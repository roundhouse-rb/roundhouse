//! Rails test-file ingestion — `test/models/*_test.rb` and
//! `test/controllers/*_test.rb`. Expects one or more test classes
//! (typically inheriting from `ActiveSupport::TestCase` or
//! `ActionDispatch::IntegrationTest`), at the top level or inside a
//! `module`, whose bodies are sequences of `test "name" do ... end`
//! declarations.

use ruby_prism::Node;

use crate::dialect::{AccessorKind, LibraryClass, MethodDef, MethodReceiver, MethodVisibility, Test, TestModule};
use crate::effect::EffectSet;
use crate::expr::{Expr, ExprNode, LValue, Literal};
use crate::ident::VarId;
use crate::span::Span;
use crate::ty::{FnTy, Ty};
use crate::{ClassId, Symbol};

use super::expr::ingest_expr;
use super::library_class::{
    ingest_library_method, library_class_from_node, library_class_from_node_with_scope,
};
use super::util::{
    class_name_path, constant_id_str, constant_path_of, flatten_statements, module_name_path,
};
use super::{IngestError, IngestResult};

/// Returns `Ok(None)` if the file doesn't contain a class (e.g. a pure
/// helper or empty file). Unrecognized top-level constructs inside the
/// class are silently skipped — same posture as `ingest_controller`'s
/// `Unknown` fallback but without the surrounding body-item model.
///
/// When the file declares multiple top-level classes — typically a
/// helper model paired with a `*Test` class (see
/// `runtime/ruby/test/action_view/view_helpers_test.rb`'s `Article` +
/// `ViewHelpersTest` pair) — pick the `*Test` class as the test
/// module and ingest the rest as helpers on the returned module's
/// `inner_classes`. This keeps the emit path generic: every per-target
/// emitter already hoists `inner_classes` above the test body, so a
/// top-level helper class lands alongside the test file without
/// per-target plumbing changes.
pub fn ingest_test_file(source: &[u8], file: &str) -> IngestResult<Option<TestModule>> {
    Ok(ingest_test_files(source, file)?.into_iter().next())
}

/// Every test class the file declares, each as its own `TestModule`.
///
/// A Rails test file is usually one class, but not always:
/// campfire's `test/channels/room_messages_channel_test.rb` declares
/// `RoomMessagesChannelTest` and, beside it, `RoomMessagesViaStock-
/// TurboChannelTest` — the second exists to prove the STOCK
/// `Turbo::StreamsChannel` turns the same stream names away, which is
/// a different subject and so a different class. `ingest_test_file`
/// kept only the first and routed the second through
/// `inner_classes`, where a `*Test` class was then skipped on
/// purpose (see the note at the end of `ingest_test_class`): two
/// tests silently absent from every tally. Each class here becomes
/// a module of its own, and the emit names files by class, so they
/// land as two test files the way Minitest would have run them as
/// two suites. Non-test top-level classes are shared by every module
/// as helpers, the way they were for the one.
pub fn ingest_test_files(source: &[u8], file: &str) -> IngestResult<Vec<TestModule>> {
    ingest_test_classes(source, file, true)
}

/// The test classes a support file (`test/test_helper.rb`) defines: only
/// classes that read as test classes, with no single-class fallback, so a
/// helper's plain `class RecordingLogger < Logger` is never mistaken for one.
pub fn ingest_test_bases(source: &[u8], file: &str) -> IngestResult<Vec<TestModule>> {
    ingest_test_classes(source, file, false)
}

fn ingest_test_classes(
    source: &[u8],
    file: &str,
    single_class_fallback: bool,
) -> IngestResult<Vec<TestModule>> {
    super::sources::register(file, &String::from_utf8_lossy(source));
    let result = super::prism::parse(source, file);
    let root = result.node();
    let mut top_classes: Vec<ScopedClass<'_>> = Vec::new();
    collect_top_level_classes(&root, &[], &mut top_classes);
    if top_classes.is_empty() {
        return Ok(Vec::new());
    }
    let (mut test_nodes, helper_nodes): (Vec<_>, Vec<_>) =
        top_classes.into_iter().partition(|(_, c)| is_test_class_node(c));
    // No candidate matches: the historical single-class shape, where
    // the first top-level class is the test whatever it is called. A
    // class inside a module is not a candidate here: a support file
    // in a test directory often declares one, and it has no tests.
    if test_nodes.is_empty() {
        if !single_class_fallback {
            return Ok(Vec::new());
        }
        let mut helper_nodes = helper_nodes;
        let Some(first) = helper_nodes.iter().position(|(scope, _)| scope.is_empty()) else {
            return Ok(Vec::new());
        };
        test_nodes.push(helper_nodes.remove(first));
        return ingest_test_class(&test_nodes[0], &helper_nodes, file).map(|tm| vec![tm]);
    }
    // A later class in this file on one of these (or reopening it) sees
    // it left out or kept. Only for this file: the caller's lists are
    // put back.
    let known = TEST_CONCERNS.with(|c| (c.borrow().unharnessed.len(), c.borrow().harnessed.len()));
    let modules: IngestResult<Vec<TestModule>> = test_nodes
        .iter()
        .map(|class| {
            let tm = ingest_test_class(class, &helper_nodes, file)?;
            let name = tm.name.0.as_str().to_string();
            TEST_CONCERNS.with(|c| {
                let mut c = c.borrow_mut();
                if tm.unharnessed.is_some() { c.unharnessed.push(name) } else { c.harnessed.push(name) }
            });
            Ok(tm)
        })
        .collect();
    TEST_CONCERNS.with(|c| {
        let mut c = c.borrow_mut();
        c.unharnessed.truncate(known.0);
        c.harnessed.truncate(known.1);
    });
    modules
}

fn ingest_test_class(
    (scope, class): &ScopedClass<'_>,
    top_level_helper_nodes: &[ScopedClass<'_>],
    file: &str,
) -> IngestResult<TestModule> {
    let mut name_path = scope.clone();
    name_path.extend(class_name_path(class).ok_or_else(|| IngestError::Unsupported {
        file: file.into(),
        message: "test class name must be a simple constant or path".into(),
    })?);
    let name = ClassId(Symbol::from(name_path.join("::")));

    let parent = class.superclass().and_then(|n| {
        constant_path_of(&n).map(|p| ClassId(Symbol::from(p.join("::"))))
    });

    // Rails convention: `ArticleTest` tests `Article`. Strip the `Test`
    // suffix off the last name-path segment. Unusual naming → None.
    let target = name_path
        .last()
        .and_then(|last| last.strip_suffix("Test"))
        .map(|stem| ClassId(Symbol::from(stem)));

    let mut acc = ClassBody { target, ..ClassBody::default() };
    // A class on a base with no harness is left out whole by the app
    // ingest (`drop_unharnessed`): its body is not read, so a construct
    // in it cannot fail the ingest that only means to report it.
    // The base as written, with the `::` of a rooted `::BaseTest` kept
    // for the lookup (the stored `parent` drops it).
    let written = class.superclass().and_then(|n| {
        let rooted = n.as_constant_path_node().is_some_and(|p| super::util::constant_path_is_rooted(&p));
        parent.as_ref().map(|p| if rooted { format!("::{}", p.0) } else { p.0.to_string() })
    });
    let unharnessed_base = TEST_CONCERNS.with(|c| {
        let c = c.borrow();
        unharnessed_class(scope, name.0.as_str(), written.as_deref(), &c.unharnessed, &c.harnessed)
    });
    let unharnessed = unharnessed_base.is_some();
    // Concerns `test_helper.rb` mixes into `ActiveSupport::TestCase`: a
    // class that derives from a framework base inherits their `included`
    // registrations ahead of its own, as a parent's callbacks run first.
    if !unharnessed && parent.as_ref().is_some_and(|p| is_framework_test_base(p.0.as_str())) {
        for concern in test_case_wide_concerns() {
            apply_test_concern(&concern, &mut acc, &name, 0)?;
        }
    }
    if let Some(class_body) = class.body().filter(|_| !unharnessed) {
        for stmt in flatten_statements(class_body) {
            if !class_body_statement(&stmt, file, &mut acc, &name, 0)? {
                report_dropped_statement(&stmt, file, &name);
            }
        }
    }
    let ClassBody { tests, setup, mut inner_classes, mut helpers, constants, includes, teardowns, target, .. } = acc;
    let body_classes = inner_classes.iter().map(|c| c.name.clone()).collect();

    // ActiveSupport runs `teardown do … end` blocks after every test,
    // passed or failed, after the class's own `def teardown` and in
    // reverse of the order they were declared. The harness calls the
    // test's `teardown` after each test the same way, so the blocks
    // become that method. An early `return` in `def teardown` must not
    // skip the blocks — wrap the method body in `ensure` so they run.
    //
    // Always `super` in a trailing ensure: TestBase's teardown runs
    // `mocha_verify` / stub-slot verifies (see `patch_stub_lifecycle`).
    // A subclass `def teardown` that omits `super` leaves Mocha expects
    // installed, and the next test fails with "unexpected invocation"
    // (campfire's CachedResponsesTest after this ingest landed).
    // Method-level `ensure` (`implicit: true`) is the outer shape; the
    // nested `begin/ensure` around the blocks is explicit so `super`
    // still runs when a block raises.
    //
    // When `def teardown` already calls `super` (including conditionally),
    // mark it at the call site and skip the ensure fallback so mocha
    // verify runs exactly once on every path.
    if !teardowns.is_empty() {
        let blocks = Expr::new(
            Span::synthetic(),
            ExprNode::Seq {
                exprs: teardowns.into_iter().rev().collect(),
            },
        );
        let syn = Span::synthetic();
        let super_call = || {
            Expr::new(
                syn,
                ExprNode::Super { args: Some(Vec::new()) },
            )
        };
        let own = helpers.iter().position(|m| m.name.as_str() == "teardown");
        let (name_span, body) = match own.map(|i| helpers.remove(i)) {
            Some(m) => {
                // def teardown
                //   __teardown_super_ran = false
                //   begin
                //     own  # Super sites set the flag then call super
                //   ensure
                //     begin
                //       blocks
                //     ensure
                //       super() unless __teardown_super_ran
                //     end
                //   end
                // end
                let flag = Symbol::from("__teardown_super_ran");
                let flag_lit = |value: bool| {
                    Expr::new(syn, ExprNode::Lit { value: Literal::Bool { value } })
                };
                let flag_var = || {
                    Expr::new(
                        syn,
                        ExprNode::Var {
                            id: VarId(0),
                            name: flag.clone(),
                        },
                    )
                };
                let assign_flag = |value: bool| {
                    Expr::new(
                        syn,
                        ExprNode::Assign {
                            target: LValue::Var {
                                id: VarId(0),
                                name: flag.clone(),
                            },
                            value: flag_lit(value),
                        },
                    )
                };
                let mut own_body = m.body;
                mark_teardown_super_ran(&mut own_body, &flag);
                let fallback_super = Expr::new(
                    syn,
                    ExprNode::If {
                        cond: flag_var(),
                        then_branch: Expr::new(syn, ExprNode::Lit { value: Literal::Nil }),
                        else_branch: super_call(),
                    },
                );
                let blocks_then_super = Expr::new(
                    syn,
                    ExprNode::BeginRescue {
                        body: blocks,
                        rescues: Vec::new(),
                        else_branch: None,
                        ensure: Some(Box::new(fallback_super)),
                        implicit: false,
                    },
                );
                (
                    m.name_span,
                    Expr::new(
                        syn,
                        ExprNode::BeginRescue {
                            body: Expr::new(
                                syn,
                                ExprNode::Seq {
                                    exprs: vec![assign_flag(false), own_body],
                                },
                            ),
                            rescues: Vec::new(),
                            else_branch: None,
                            ensure: Some(Box::new(blocks_then_super)),
                            implicit: true,
                        },
                    ),
                )
            }
            None => (
                Span::synthetic(),
                // def teardown
                //   blocks
                // ensure
                //   super()
                // end
                Expr::new(
                    Span::synthetic(),
                    ExprNode::BeginRescue {
                        body: blocks,
                        rescues: Vec::new(),
                        else_branch: None,
                        ensure: Some(Box::new(super_call())),
                        implicit: true,
                    },
                ),
            ),
        };
        helpers.push(MethodDef {
            name_span,
            name: Symbol::from("teardown"),
            receiver: crate::dialect::MethodReceiver::Instance,
            visibility: crate::dialect::MethodVisibility::Public,
            params: Vec::new(),
            unsupported_formals: None,
            has_anonymous_block: false,
            body,
            signature: None,
            effects: crate::effect::EffectSet::default(),
            enclosing_class: Some(name.0.clone()),
            kind: crate::dialect::AccessorKind::Method,
            is_async: false,
            mutates_self: false,
            block_param: None,
        });
    }

    // Top-level helper classes (e.g. `class Article < ActiveRecord::
    // Base` declared next to `ViewHelpersTest` in the same file) ingest
    // as LibraryClass and flow through the same `inner_classes` channel
    // as body-internal helpers. The per-target emit paths already hoist
    // `inner_classes` to file scope above the test body — same routing,
    // no further per-target changes needed.
    //
    // Additional `*Test`-shaped classes are each a module of their
    // own — `ingest_test_files` splits them before this runs — so
    // every node here is a genuine helper. (They used to be dropped:
    // an inner class does not get the `< Minitest::Test → < TestBase`
    // parent rewrite the test class does, so emitting one inline
    // would have Minitest find it under CRuby and run it against the
    // wrong assertion surface.)
    for (helper_scope, helper_node) in top_level_helper_nodes {
        let lc = library_class_from_node_with_scope(helper_node, helper_scope, file)?;
        inner_classes.push(lc);
    }

    Ok(TestModule {
        name,
        parent,
        target,
        tests,
        setup,
        inner_classes,
        body_classes,
        helpers,
        constants,
        includes,
        unharnessed: unharnessed_base.map(|b| ClassId(Symbol::from(b.as_str()))),
    })
}

/// A test class body statement the ingest does not model, ledgered by
/// name: the emitted class goes without it (`INPUT[:accent] = ACCENT`
/// between two constants, so a later constant built from `INPUT` misses
/// the key).
/// A bare `private` is not one: the helpers below it are called from the
/// class's own tests either way.
fn report_dropped_statement(stmt: &Node, file: &str, class: &ClassId) {
    let construct = match stmt.as_call_node() {
        Some(call) => constant_id_str(&call.name()).to_string(),
        None => "statement".to_string(),
    };
    let bare_marker = stmt.as_call_node().is_some_and(|call| call.receiver().is_none() && call.arguments().is_none());
    if bare_marker && matches!(construct.as_str(), "private" | "protected" | "public") {
        return;
    }
    let line = String::from_utf8_lossy(stmt.location().as_slice()).lines().next().unwrap_or("").to_string();
    super::prism::push(crate::lower::residue_diagnostic(
        "test_class_body",
        &construct,
        super::util::node_span(stmt, file),
        "not modelled",
        format!("`{line}` in the body of test class `{}` is not modelled and is dropped", class.0.as_str()),
    ));
}

/// `self.use_transactional_tests = false` (or `true`) as
/// `def self.use_transactional_tests; <bool>; end`. The shim calls that
/// class method; `TestBase` answers true when the class does not override it.
fn transactional_tests_method(stmt: &Node) -> Option<MethodDef> {
    let call = stmt.as_call_node()?;
    if constant_id_str(&call.name()) != "use_transactional_tests=" {
        return None;
    }
    let recv = call.receiver()?;
    if recv.as_self_node().is_none() {
        return None;
    }
    let args = call.arguments()?;
    let arg = args.arguments().iter().next()?;
    let value = if arg.as_true_node().is_some() {
        true
    } else if arg.as_false_node().is_some() {
        false
    } else {
        return None;
    };
    let body = Expr::new(
        Span::synthetic(),
        ExprNode::Lit {
            value: Literal::Bool { value },
        },
    );
    Some(MethodDef {
        name: Symbol::from("use_transactional_tests"),
        receiver: MethodReceiver::Class,
        visibility: MethodVisibility::Public,
        params: Vec::new(),
        unsupported_formals: None,
        has_anonymous_block: false,
        block_param: None,
        name_span: Span::synthetic(),
        body,
        signature: Some(Ty::Fn(Box::new(FnTy {
            params: Vec::new().into(),
            block: None,
            ret: std::sync::Arc::new(Ty::Bool),
            effects: EffectSet::pure(),
        }))),
        effects: EffectSet::pure(),
        enclosing_class: None,
        kind: AccessorKind::Method,
        is_async: false,
        mutates_self: false,
    })
}

/// The app-wide `setup do … end` an app's `test/test_helper.rb` puts on
/// `ActiveSupport::TestCase` (reopened there, or nested as `module
/// ActiveSupport; class TestCase`). Rails runs it before every test
/// case's own setup, so the caller splices it ahead of each test
/// module's `setup` — the same move `splice_test_helpers` makes for the
/// helper modules that file includes.
///
/// campfire's is three statements — clear the cable test adapter,
/// replace the web-push pool, forbid network — and the middle one is
/// why this exists: `Room::PushTest` waits on the pool's task counter,
/// which is cumulative, so every test needs the fresh pool the app's
/// own setup gives it. Without the splice the suite's first push test
/// leaves a count the next one's wait is satisfied by before its tasks
/// have run.
///
/// Only the `setup` hook is read. `fixtures :all`, `parallelize`, and
/// the `include` list are the harness's concern (the includes are read
/// by `ingest_test_helper_modules`), and a `teardown` there has no
/// home yet — the emitted `TestBase#teardown` is ours.
pub fn ingest_test_case_setup(source: &[u8], file: &str) -> IngestResult<Option<Expr>> {
    let result = super::prism::parse(source, file);
    let root = result.node();
    let mut classes: Vec<ScopedClass<'_>> = Vec::new();
    collect_top_level_classes(&root, &[], &mut classes);
    for (_, class) in classes {
        let Some(path) = class_name_path(&class) else { continue };
        if path.last().map(String::as_str) != Some("TestCase") {
            continue;
        }
        let Some(body) = class.body() else { continue };
        let mut setup: Option<Expr> = None;
        let mut after_fixtures: Vec<Expr> = Vec::new();
        for stmt in flatten_statements(body) {
            if setup.is_none() {
                if let Some(found) = ingest_setup_declaration(&stmt, file)? {
                    setup = Some(found);
                    continue;
                }
            }
            if let Some(stmts) = ingest_load_fixtures_override(&stmt, file)? {
                after_fixtures = stmts;
            }
        }
        if after_fixtures.is_empty() {
            return Ok(setup);
        }
        // The fixture hook's statements run first: Rails loads fixtures
        // in `before_setup`, ahead of every `setup` block, and the
        // harness reloads them per test before this runs.
        let mut exprs = after_fixtures;
        match setup {
            Some(s) => match *s.node {
                ExprNode::Seq { exprs: own } => exprs.extend(own),
                node => exprs.push(Expr::new(s.span, node)),
            },
            None => {}
        }
        return Ok(Some(Expr::new(Span::synthetic(), ExprNode::Seq { exprs })));
    }
    Ok(None)
}

/// Rewrite every `super` in a teardown body to set `__teardown_super_ran`
/// before the call, so the ensure fallback can skip a second inherited
/// teardown. Works for bare `super`, `super(...)`, and conditional forms
/// (`super if x`) — only executed Super sites flip the flag.
fn mark_teardown_super_ran(expr: &mut Expr, flag: &Symbol) {
    if matches!(&*expr.node, ExprNode::Super { .. }) {
        let super_expr = std::mem::replace(
            expr,
            Expr::new(Span::synthetic(), ExprNode::Lit { value: Literal::Nil }),
        );
        *expr = Expr::new(
            super_expr.span,
            ExprNode::Seq {
                exprs: vec![
                    Expr::new(
                        Span::synthetic(),
                        ExprNode::Assign {
                            target: LValue::Var {
                                id: VarId(0),
                                name: flag.clone(),
                            },
                            value: Expr::new(
                                Span::synthetic(),
                                ExprNode::Lit {
                                    value: Literal::Bool { value: true },
                                },
                            ),
                        },
                    ),
                    super_expr,
                ],
            },
        );
        return;
    }
    expr.node
        .for_each_child_mut(&mut |child| mark_teardown_super_ran(child, flag));
}

/// `def load_fixtures(config) = (fixtures = super; …; fixtures)` on the
/// app's `ActiveSupport::TestCase` — work to do once fixtures are in
/// the database. campfire repairs `rooms.messages_count` there, because
/// `fixtures :all` inserts messages before the rooms they count:
///
/// ```ruby
/// def load_fixtures(config)
///   fixtures = super
///   Room::MessagesCount.ensure!
///   Room::MessagesCount.backfill!
///   fixtures
/// end
/// ```
///
/// Only that shape — `super` first (bare or assigned), its value last
/// when assigned — is read, as the statements between. The harness
/// loads fixtures itself, so those statements are what the override
/// adds. Any other shape is an error rather than a hook silently left
/// out.
fn ingest_load_fixtures_override(stmt: &Node<'_>, file: &str) -> IngestResult<Option<Vec<Expr>>> {
    let Some(def) = stmt.as_def_node() else { return Ok(None) };
    if std::str::from_utf8(def.name().as_slice()).ok() != Some("load_fixtures") {
        return Ok(None);
    }
    let unsupported = || IngestError::Unsupported {
        file: file.into(),
        message: "a `load_fixtures` override other than `x = super; …; x` is not supported".into(),
    };
    let Some(body) = def.body() else { return Ok(Some(Vec::new())) };
    let body = ingest_expr(&body, file)?;
    let mut stmts = match *body.node {
        ExprNode::Seq { exprs } => exprs,
        node => vec![Expr::new(body.span, node)],
    };
    if stmts.is_empty() {
        return Err(unsupported());
    }
    let first = stmts.remove(0);
    let is_super = |e: &Expr| matches!(&*e.node, ExprNode::Super { .. });
    match &*first.node {
        _ if is_super(&first) => {}
        ExprNode::Assign { target: crate::expr::LValue::Var { name, .. }, value } if is_super(value) => {
            let returns_it = stmts
                .last()
                .is_some_and(|e| matches!(&*e.node, ExprNode::Var { name: n, .. } if n == name));
            if !returns_it {
                return Err(unsupported());
            }
            stmts.pop();
        }
        _ => return Err(unsupported()),
    }
    Ok(Some(stmts))
}

/// Collect every class the file declares outside another class, each
/// with the path of the modules that enclose it. `module Models; class
/// PostTest` gives `(["Models"], PostTest)`, the same class as `class
/// Models::PostTest`. The walk does not go into class bodies: a class
/// inside a test class is a helper of that test class.
fn collect_top_level_classes<'pr>(
    node: &Node<'pr>,
    scope: &[String],
    out: &mut Vec<ScopedClass<'pr>>,
) {
    if let Some(c) = node.as_class_node() {
        out.push((scope.to_vec(), c));
        return;
    }
    if let Some(p) = node.as_program_node() {
        collect_top_level_classes(&p.statements().as_node(), scope, out);
        return;
    }
    if let Some(s) = node.as_statements_node() {
        for stmt in s.body().iter() {
            collect_top_level_classes(&stmt, scope, out);
        }
        return;
    }
    if let Some(m) = node.as_module_node() {
        let mut inner = scope.to_vec();
        inner.extend(module_name_path(&m).unwrap_or_default());
        if let Some(body) = m.body() {
            collect_top_level_classes(&body, &inner, out);
        }
    }
}

type ScopedClass<'pr> = (Vec<String>, ruby_prism::ClassNode<'pr>);

/// Heuristic: a class is "the test class" when its name ends with
/// `Test` (e.g. `ViewHelpersTest`, `InflectorTest`) OR when its parent
/// path's leaf segment is a known test base (`Test`, `TestCase`,
/// `IntegrationTest`). The two conditions together cover both the
/// framework's own `< Minitest::Test` shape and Rails app tests'
/// `< ActiveSupport::TestCase` / `< ActionDispatch::IntegrationTest`.
fn is_test_class_node(c: &ruby_prism::ClassNode<'_>) -> bool {
    if let Some(name_path) = class_name_path(c) {
        if let Some(last) = name_path.last() {
            if last.ends_with("Test") {
                return true;
            }
        }
    }
    if let Some(parent_node) = c.superclass() {
        if let Some(parent_path) = constant_path_of(&parent_node) {
            if let Some(last) = parent_path.last() {
                if matches!(last.as_str(), "Test" | "TestCase" | "IntegrationTest") {
                    return true;
                }
            }
        }
    }
    false
}

/// Recognize `setup do ... end` (Call with method=setup, block body)
/// and `def setup; ...; end` (DefNode named setup). Returns the body
/// expression in either case. Other shapes return Ok(None).
/// What a test class body declares, gathered statement by statement:
/// the class's own and, through `apply_test_concern`, the statements of
/// a test helper Concern's `included` block where it is included.
#[derive(Default)]
struct ClassBody {
    tests: Vec<Test>,
    /// Each test's Ruby method name, beside it in `tests`
    /// (`test "a b"` is `test_a_b`, `def test_a__b` is itself).
    test_methods: Vec<String>,
    setup: Option<Expr>,
    inner_classes: Vec<crate::dialect::LibraryClass>,
    helpers: Vec<MethodDef>,
    constants: Vec<(Symbol, Expr)>,
    includes: Vec<ClassId>,
    teardowns: Vec<Expr>,
    target: Option<ClassId>,
    /// Concerns whose `included` block already ran on this class: Ruby
    /// includes a module once, so a second `include` runs nothing.
    applied: Vec<String>,
}

/// One statement of a test class body. Returns whether it is one the
/// class body models; the class's own unrecognized statements are
/// skipped, as they always were, and a Concern block's are refused.
fn class_body_statement(
    stmt: &Node<'_>,
    file: &str,
    acc: &mut ClassBody,
    name: &ClassId,
    depth: usize,
) -> IngestResult<bool> {
    if let Some(test) = ingest_test_declaration(&stmt, file)? {
        let method = match stmt.as_def_node() {
            Some(def) => constant_id_str(&def.name()).to_string(),
            None => test_method_name(&test.name),
        };
        if let Err(err) = add_test(acc, test, method, stmt.as_def_node().is_some(), file) {
            super::survey::continue_or_fail(err)?;
        }
        return Ok(true);
    }
    if let Some(body) = ingest_teardown_block(&stmt, file)? {
        acc.teardowns.push(body);
        return Ok(true);
    }
    if let Some(body) = ingest_setup_hoisting(&stmt, file, Some(&mut acc.inner_classes))? {
        // Multiple setup hooks compose in source order — append
        // by wrapping in a Seq. Rare in practice; first wins
        // when stored as Option, so accumulate.
        acc.setup = Some(match acc.setup.take() {
            None => body,
            Some(prev) => {
                let prev_stmts = match &*prev.node {
                    ExprNode::Seq { exprs } => exprs.clone(),
                    _ => vec![prev],
                };
                let body_stmts = match &*body.node {
                    ExprNode::Seq { exprs } => exprs.clone(),
                    _ => vec![body],
                };
                let mut all = prev_stmts;
                all.extend(body_stmts);
                Expr::new(Span::synthetic(), ExprNode::Seq { exprs: all })
            }
        });
        return Ok(true);
    }
    // Inline class declaration inside the test class —
    // `class Validatable; include ActiveRecord::Validations;
    // ...; end`. Captured per-test-module so the emit can
    // hoist them to file scope above the lowered test class.
    if let Some(inner) = stmt.as_class_node() {
        let lc = library_class_from_node(&inner, file)?;
        // Not a second definition beside a setup's hoisted one: Ruby runs the body before setup, so setup's wins.
        if acc.inner_classes.iter().any(|c| c.name == lc.name) {
            return Err(IngestError::Unsupported {
                file: file.into(),
                message: format!("class `{}` is defined twice in the test class", lc.name.0),
            });
        }
        acc.inner_classes.push(lc);
        return Ok(true);
    }
    // Non-test, non-setup `def` — instance helper method
    // (e.g. `setup_adapter_with_stub_row(id)`). Capture as
    // an ordinary method on the test class so test bodies
    // can `self.<helper>(...)` it. Definitions of `def
    // setup` and `def test_*` already short-circuited above.
    if let Some(def) = stmt.as_def_node() {
        let m = ingest_library_method(&def, name, file)?;
        acc.helpers.push(m);
        return Ok(true);
    }
    // `{ … }.each do |name, url| test "#{name} …" do … end end`: the
    // tests a class-body loop declares, one per element. A loop it
    // cannot unroll is reported, not skipped with its tests.
    match ingest_test_loop(stmt, file) {
        Ok(None) => {}
        Ok(Some(tests)) => {
            // All of the loop's tests or none of them.
            let before = acc.tests.len();
            for test in tests {
                let method = test_method_name(&test.name);
                if let Err(err) = add_test(acc, test, method, false, file) {
                    acc.tests.truncate(before);
                    acc.test_methods.truncate(before);
                    super::survey::continue_or_fail(err)?;
                    break;
                }
            }
            return Ok(true);
        }
        Err(err) => {
            super::survey::continue_or_fail(err)?;
            return Ok(true);
        }
    }
    // A `test` call `ingest_test_declaration` did not read: its name is
    // not a string literal (`test "#{KIND} works"`).
    if declares_test(stmt) {
        super::survey::continue_or_fail(IngestError::Unsupported {
            file: file.into(),
            message: "a test whose name is not a string literal is not emitted".into(),
        })?;
        return Ok(true);
    }
    // `include ModName` at class-body scope. Captured for
    // the spinel emit to replay verbatim so bare-name refs
    // (`Router`, `FormBuilder`) resolve under CRuby. TS emit
    // ignores these — its framework-namespace import-stripper
    // handles the same refs by a different mechanism.
    if let Some(call) = stmt.as_call_node() {
        // `tests RoomMessagesChannel` — ActionCable's channel
        // and connection test cases (and ActionController::
        // TestCase) name their subject with this macro when
        // the class name does not: `RoomMessagesViaStock-
        // TurboChannelTest` tests `Turbo::StreamsChannel`. It
        // overrides the `FooTest` → `Foo` convention above.
        if call.receiver().is_none() && constant_id_str(&call.name()) == "tests" {
            if let Some(args) = call.arguments() {
                if let Some(arg) = args.arguments().iter().next() {
                    if let Some(path) = constant_path_of(&arg) {
                        acc.target = Some(ClassId(Symbol::from(path.join("::"))));
                    }
                }
            }
            return Ok(true);
        }
        if call.receiver().is_none()
            && constant_id_str(&call.name()) == "include"
        {
            if let Some(args) = call.arguments() {
                for arg in args.arguments().iter() {
                    if let Some(path) = constant_path_of(&arg) {
                        let included = path.join("::");
                        acc.includes.push(ClassId(Symbol::from(included.as_str())));
                        if let Some(concern) = test_concern(&included) {
                            apply_test_concern(&concern, acc, name, depth)?;
                        }
                    }
                }
            }
            return Ok(true);
        }
    }
    // `self.use_transactional_tests = false` — Rails' per-class
    // opt-out of the fixture transaction. Kept as a class method
    // so the autorun shim can ask the class. Anything else in
    // the class body is still skipped.
    if let Some(method) = transactional_tests_method(&stmt) {
        acc.helpers.push(method);
        return Ok(true);
    }
    // Class-level constant assignment — `TABLE = [...]`,
    // `Article = Struct.new(...)`. Captured here so the test
    // emit can hoist them to file scope as `const NAME =
    // <value>`, which matches Ruby's lexical constant
    // resolution for bare references inside test methods.
    if let Some(cw) = stmt.as_constant_write_node() {
        let const_name = std::str::from_utf8(cw.name().as_slice())
            .ok()
            .map(Symbol::from);
        if let Some(const_name) = const_name {
            let value = ingest_expr(&cw.value(), file)?;
            acc.constants.push((const_name, value));
        }
        return Ok(true);
    }
    // Tests declared inside a statement it does not model (`if ENV[…]`,
    // `define_method("test_#{x}")`): reported, never skipped with it.
    if contains_test(stmt) {
        super::survey::continue_or_fail(IngestError::Unsupported {
            file: file.into(),
            message: "tests declared inside this class-body statement are not emitted".into(),
        })?;
        return Ok(true);
    }

    Ok(false)
}

/// A test helper `ActiveSupport::Concern` (`test/test_helpers/`, or a
/// module-only file in a test folder) whose `included do` block a test
/// class runs when it includes the module.
#[derive(Clone, Debug)]
pub struct TestConcern {
    pub name: String,
    pub file: String,
    pub source: Vec<u8>,
}

/// The app's test helper Concerns, and the names `test/test_helper.rb`
/// includes into `ActiveSupport::TestCase` (every test case's).
#[derive(Clone, Debug, Default)]
pub struct TestConcerns {
    pub concerns: Vec<TestConcern>,
    pub case_wide: Vec<String>,
    /// App test bases left out for a base with no harness (one
    /// `test/test_helper.rb` defines on `ActionMailer::TestCase`): a
    /// class on one has its body skipped as on a framework one.
    pub unharnessed: Vec<String>,
    /// Test classes kept so far: a base resolved to one has a harness,
    /// and shadows a left-out class of that name further out.
    pub harnessed: Vec<String>,
}

thread_local! {
    static TEST_CONCERNS: std::cell::RefCell<TestConcerns> = std::cell::RefCell::new(TestConcerns::default());
}

/// Ingest test files with `concerns` known: an `include` of one runs its
/// `included` block's statements on the including class.
pub fn with_test_concerns<R>(concerns: TestConcerns, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<TestConcerns>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let prev = self.0.take().unwrap_or_default();
            TEST_CONCERNS.with(|c| *c.borrow_mut() = prev);
        }
    }
    let _restore = Restore(Some(TEST_CONCERNS.with(|c| c.replace(concerns))));
    f()
}

fn test_concern(name: &str) -> Option<TestConcern> {
    let name = name.trim_start_matches("::");
    TEST_CONCERNS.with(|c| c.borrow().concerns.iter().find(|tc| tc.name == name).cloned())
}

fn test_case_wide_concerns() -> Vec<TestConcern> {
    TEST_CONCERNS.with(|c| {
        let c = c.borrow();
        c.case_wide.iter().filter_map(|n| c.concerns.iter().find(|tc| &tc.name == n).cloned()).collect()
    })
}

/// Framework test-case bases the emitted test harness does not define
/// (`runtime/spinel/test/test_helper.rb` has `ActiveSupport::TestCase`'s,
/// the integration, controller, view and cable ones).
pub(crate) const UNHARNESSED_TEST_BASES: [&str; 4] = [
    "ActionMailer::TestCase",
    "ActiveJob::TestCase",
    "ActionMailbox::TestCase",
    "ActionDispatch::SystemTestCase",
];

/// The base test class `name` (on `parent` as written, inside the modules
/// `scope`) is left out for, or `name` itself when it is in `left_out`
/// already (a reopen): a framework base in [`UNHARNESSED_TEST_BASES`], or
/// a class in `left_out`. A parent written relative to the enclosing
/// modules (`Base` inside `module Mailers`) is looked up innermost module
/// first, approximately as Ruby does: the first candidate in `left_out`
/// or in `kept` (test classes with a harness) decides, so a kept inner
/// class shadows a left-out outer one.
fn unharnessed_class(
    scope: &[String],
    name: &str,
    parent: Option<&str>,
    left_out: &[String],
    kept: &[String],
) -> Option<String> {
    let is_left_out = |class: &str| left_out.iter().any(|u| u == class);
    if is_left_out(name) {
        return Some(name.to_string());
    }
    let parent = parent?;
    if let Some(rooted) = parent.strip_prefix("::") {
        return (UNHARNESSED_TEST_BASES.contains(&rooted) || is_left_out(rooted)).then(|| rooted.to_string());
    }
    for depth in (0..=scope.len()).rev() {
        let candidate = scope[..depth].iter().map(String::as_str).chain([parent]).collect::<Vec<_>>().join("::");
        if is_left_out(&candidate) {
            return Some(candidate);
        }
        if kept.contains(&candidate) {
            return None;
        }
    }
    UNHARNESSED_TEST_BASES.contains(&parent).then(|| parent.to_string())
}

/// A framework test case: one `ActiveSupport::TestCase` is, or derives
/// from, so a Concern included into it reaches the class.
fn is_framework_test_base(parent: &str) -> bool {
    matches!(
        parent.trim_start_matches("::"),
        "ActiveSupport::TestCase"
            | "ActionDispatch::IntegrationTest"
            | "ActionController::TestCase"
            | "ActionMailer::TestCase"
            | "ActionView::TestCase"
            | "ActiveJob::TestCase"
            | "ActionCable::Channel::TestCase"
            | "ActionCable::Connection::TestCase"
            | "ActionDispatch::SystemTestCase"
    )
}

/// Every module in `source` that is a Concern with an `included do`
/// block, by its full name (`Widgets::ReadHelper`).
pub fn concern_modules(source: &[u8], file: &str) -> Vec<String> {
    let result = super::prism::parse(source, file);
    let mut out = Vec::new();
    fn walk(node: &Node<'_>, scope: &[String], out: &mut Vec<String>) {
        let Some(stmts) = node.as_statements_node() else { return };
        for stmt in stmts.body().iter() {
            let Some(m) = stmt.as_module_node() else { continue };
            let Some(path) = module_name_path(&m) else { continue };
            let mut full = scope.to_vec();
            full.extend(path);
            if included_block_body(&m).is_some() {
                out.push(full.join("::"));
            }
            if let Some(body) = m.body() {
                walk(&body, &full, out);
            }
        }
    }
    if let Some(program) = result.node().as_program_node() {
        walk(&program.statements().as_node(), &[], &mut out);
    }
    out
}

/// The body of a Concern module's `included do … end`: the module must
/// `extend ActiveSupport::Concern`, which is what gives `included` a
/// block to run on the includer.
fn included_block_body<'pr>(m: &ruby_prism::ModuleNode<'pr>) -> Option<Option<Node<'pr>>> {
    let body = m.body()?;
    let stmts = flatten_statements(body);
    let concern = stmts.iter().any(|stmt| {
        stmt.as_call_node().is_some_and(|call| {
            call.receiver().is_none()
                && constant_id_str(&call.name()) == "extend"
                && call.arguments().is_some_and(|args| {
                    args.arguments().iter().any(|a| {
                        constant_path_of(&a).is_some_and(|p| p.join("::").trim_start_matches("::") == "ActiveSupport::Concern")
                    })
                })
        })
    });
    if !concern {
        return None;
    }
    stmts.iter().find_map(|stmt| {
        let call = stmt.as_call_node()?;
        if call.receiver().is_some() || constant_id_str(&call.name()) != "included" || call.arguments().is_some() {
            return None;
        }
        let block = call.block()?.as_block_node()?;
        Some(block.body())
    })
}

/// `Concern#append_features`: the `included` block runs on the
/// includer where the `include` stands. Its statements go through the
/// same class-body reading as the class's own, and one the class body
/// does not model is refused by name rather than dropped, since the
/// registration it made would otherwise never run.
fn apply_test_concern(concern: &TestConcern, acc: &mut ClassBody, name: &ClassId, depth: usize) -> IngestResult<()> {
    if acc.applied.iter().any(|n| n == &concern.name) {
        return Ok(());
    }
    acc.applied.push(concern.name.clone());
    let refuse = |message: String| IngestError::Unsupported { file: concern.file.clone(), message };
    if depth > 8 {
        return Err(refuse(format!("test helper `{}`: included blocks nest too deeply", concern.name)));
    }
    let result = super::prism::parse(&concern.source, &concern.file);
    let path: Vec<&str> = concern.name.split("::").collect();
    fn find<'pr>(node: &Node<'pr>, path: &[&str]) -> Option<ruby_prism::ModuleNode<'pr>> {
        let stmts = node.as_statements_node()?;
        for stmt in stmts.body().iter() {
            let Some(m) = stmt.as_module_node() else { continue };
            let Some(own) = module_name_path(&m) else { continue };
            if path.len() < own.len() || path[..own.len()].iter().zip(&own).any(|(a, b)| a != b) {
                continue;
            }
            if path.len() == own.len() {
                return Some(m);
            }
            if let Some(found) = m.body().and_then(|body| find(&body, &path[own.len()..])) {
                return Some(found);
            }
        }
        None
    }
    let program = result.node();
    let Some(module) = program.as_program_node().and_then(|p| find(&p.statements().as_node(), &path)) else {
        return Ok(());
    };
    let Some(body) = included_block_body(&module) else { return Ok(()) };
    for stmt in body.map(flatten_statements).unwrap_or_default() {
        if !class_body_statement(&stmt, &concern.file, acc, name, depth + 1)? {
            let line = String::from_utf8_lossy(stmt.location().as_slice()).lines().next().unwrap_or("").to_string();
            return Err(refuse(format!(
                "test helper `{}`'s included block: `{line}` is not modeled on a test class",
                concern.name
            )));
        }
    }
    Ok(())
}

/// `setup :a, :b` / `teardown :a`: callbacks naming the test's own
/// methods, run in the order named. Options (`prepend: true`, `if:`)
/// are refused by name.
fn named_callbacks(call: &ruby_prism::CallNode<'_>, kind: &str, file: &str) -> IngestResult<Expr> {
    let mut exprs = Vec::new();
    for arg in call.arguments().into_iter().flat_map(|a| a.arguments().iter()) {
        let Some(sym) = arg.as_symbol_node() else {
            return Err(IngestError::Unsupported {
                file: file.into(),
                message: format!("test {kind} not modeled: only method names (`{kind} :name`) are, not options"),
            });
        };
        let method = Symbol::from(String::from_utf8_lossy(sym.unescaped()).as_ref());
        exprs.push(Expr::new(
            Span::synthetic(),
            ExprNode::Send { recv: None, method, args: vec![], block: None, parenthesized: false },
        ));
    }
    Ok(Expr::new(Span::synthetic(), ExprNode::Seq { exprs }))
}

fn ingest_setup_declaration(
    stmt: &Node<'_>,
    file: &str,
) -> IngestResult<Option<Expr>> {
    ingest_setup_hoisting(stmt, file, None)
}

/// A class whose body only declares (`include`, `extend`, `attr_*`, `def`): one `setup do` may define, since running
/// it again reopens the same class to the same effect. Ruby puts it in the test class's lexical scope, as the class
/// body's own inline classes are.
fn static_class_body(class: &ruby_prism::ClassNode<'_>) -> bool {
    let Some(body) = class.body() else { return true };
    flatten_statements(body).iter().all(|stmt| {
        stmt.as_def_node().is_some()
            || stmt.as_call_node().is_some_and(|call| {
                call.receiver().is_none()
                    && call.block().is_none()
                    && matches!(constant_id_str(&call.name()).as_ref(), "include" | "extend" | "attr_reader" | "attr_writer" | "attr_accessor")
            })
    })
}

fn ingest_setup_hoisting(
    stmt: &Node<'_>,
    file: &str,
    mut hoisted: Option<&mut Vec<LibraryClass>>,
) -> IngestResult<Option<Expr>> {
    // `setup do ... end` form.
    if let Some(call) = stmt.as_call_node() {
        if call.receiver().is_some() {
            return Ok(None);
        }
        if constant_id_str(&call.name()) != "setup" {
            return Ok(None);
        }
        if let Some(block_ref) = call.block() {
            if let Some(block) = block_ref.as_block_node() {
                let body = match block.body() {
                    Some(body_node) => match hoisted.as_deref_mut() {
                        // maybe's `setup do class MockProcessor; include PlaidAccount::TypeMappable; end … end`.
                        Some(sink) => {
                            let mut exprs = Vec::new();
                            for inner in flatten_statements(body_node) {
                                match inner.as_class_node() {
                                    // Not a second definition of a class one already holds: run in order, the two
                                    // would redefine its methods between them.
                                    Some(class) if static_class_body(&class) && {
                                        let lc = library_class_from_node(&class, file)?;
                                        let fresh = !sink.iter().any(|c| c.name == lc.name);
                                        if fresh {
                                            sink.push(lc);
                                        }
                                        fresh
                                    } => {}
                                    _ => exprs.push(ingest_expr(&inner, file)?),
                                }
                            }
                            Expr::new(Span::synthetic(), ExprNode::Seq { exprs })
                        }
                        None => ingest_expr(&body_node, file)?,
                    },
                    None => Expr::new(Span::synthetic(), ExprNode::Seq { exprs: vec![] }),
                };
                return Ok(Some(body));
            }
        }
        if call.block().is_none() && call.arguments().is_some() {
            return named_callbacks(&call, "setup", file).map(Some);
        }
        return Ok(None);
    }
    // `def setup; ...; end` form.
    if let Some(def) = stmt.as_def_node() {
        let name_bytes = def.name().as_slice();
        if std::str::from_utf8(name_bytes).ok() != Some("setup") {
            return Ok(None);
        }
        super::forwarding::reject_entrypoint(&def, file, "test setup hook")?;
        let body = match def.body() {
            Some(body_node) => ingest_expr(&body_node, file)?,
            None => Expr::new(Span::synthetic(), ExprNode::Seq { exprs: vec![] }),
        };
        return Ok(Some(body));
    }
    Ok(None)
}

/// `teardown do … end`: the block's body. Any other `teardown` call
/// (`teardown :close_files`, a block argument, `self.teardown do`) is
/// refused by name: it would otherwise fall to the unrecognized-statement
/// drop below and its cleanup would silently never run.
fn ingest_teardown_block(stmt: &Node<'_>, file: &str) -> IngestResult<Option<Expr>> {
    let Some(call) = stmt.as_call_node() else { return Ok(None) };
    if constant_id_str(&call.name()) != "teardown" {
        return Ok(None);
    }
    // Receiverless `teardown do` is the ActiveSupport form. `self.teardown
    // do` is the same callback in some suites; refuse it rather than drop.
    if let Some(recv) = call.receiver() {
        let is_self = recv.as_self_node().is_some();
        if is_self {
            return Err(IngestError::Unsupported {
                file: file.into(),
                message: "test teardown not modeled: only receiverless `teardown do … end` runs after each test".into(),
            });
        }
        return Ok(None);
    }
    if call.block().is_none() && call.arguments().is_some() {
        return named_callbacks(&call, "teardown", file).map(Some);
    }
    let block = call.block().and_then(|b| b.as_block_node());
    match block {
        Some(block) if call.arguments().is_none() => Ok(Some(match block.body() {
            Some(body_node) => ingest_expr(&body_node, file)?,
            None => Expr::new(Span::synthetic(), ExprNode::Seq { exprs: vec![] }),
        })),
        _ => Err(IngestError::Unsupported {
            file: file.into(),
            message: "test teardown not modeled: only `teardown do … end` runs after each test".into(),
        }),
    }
}

/// Recognize a single test declaration. Two shapes are supported:
///
///   `test "name" do ... end`  — Rails AS::TestCase DSL. Test name is the
///                                string argument verbatim.
///   `def test_*; ...; end`    — vanilla Minitest. Test name is the
///                                method name with the `test_` prefix
///                                stripped and underscores replaced with
///                                spaces (mirrors how Minitest reports
///                                them; keeps the two shapes
///                                indistinguishable downstream).
///
/// Returns `None` for any other statement shape (intentional — we
/// silently drop unrecognized body items rather than error; they'll
/// surface when the test tries to reference something that isn't
/// emitted).
fn ingest_test_declaration(
    stmt: &Node<'_>,
    file: &str,
) -> IngestResult<Option<Test>> {
    // `def test_*; ...; end` form (vanilla Minitest). `def self.test_order`
    // is a class method, not a test.
    if let Some(def) = stmt.as_def_node().filter(|d| d.receiver().is_none()) {
        let name_bytes = def.name().as_slice();
        let Some(method_name) = std::str::from_utf8(name_bytes).ok() else {
            return Ok(None);
        };
        let Some(stem) = method_name.strip_prefix("test_") else {
            return Ok(None);
        };
        super::forwarding::reject_entrypoint(&def, file, "test entrypoint")?;
        let name = stem.replace('_', " ");
        let body = match def.body() {
            Some(body_node) => ingest_expr(&body_node, file)?,
            None => Expr::new(Span::synthetic(), ExprNode::Seq { exprs: vec![] }),
        };
        return Ok(Some(Test { name, body }));
    }

    let Some(call) = stmt.as_call_node() else {
        return Ok(None);
    };
    // Must be a bare-name `test(...)` call.
    if call.receiver().is_some() {
        return Ok(None);
    }
    let method = constant_id_str(&call.name()).to_string();
    if method != "test" {
        return Ok(None);
    }

    // First positional argument: the test's name as a string literal.
    let Some(args) = call.arguments() else {
        return Ok(None);
    };
    let args_iter = args.arguments();
    let Some(first_arg) = args_iter.iter().next() else {
        return Ok(None);
    };
    let Some(string_node) = first_arg.as_string_node() else {
        return Ok(None);
    };
    let name = String::from_utf8_lossy(string_node.unescaped()).into_owned();

    // The block body — Prism exposes it via `call.block()` when present.
    let body = match call.block() {
        Some(block_ref) => match block_ref.as_block_node() {
            Some(block) => match block.body() {
                Some(body_node) => ingest_expr(&body_node, file)?,
                None => Expr::new(Span::synthetic(), ExprNode::Seq { exprs: vec![] }),
            },
            None => Expr::new(Span::synthetic(), ExprNode::Seq { exprs: vec![] }),
        },
        None => Expr::new(Span::synthetic(), ExprNode::Seq { exprs: vec![] }),
    };

    Ok(Some(Test { name, body }))
}

/// Whether `stmt` declares a test: `test "name" do` or `def test_name`.
fn declares_test(stmt: &Node<'_>) -> bool {
    if let Some(def) = stmt.as_def_node() {
        return constant_id_str(&def.name()).starts_with("test_");
    }
    stmt.as_call_node()
        .is_some_and(|c| c.receiver().is_none() && constant_id_str(&c.name()) == "test")
}

/// Whether `node` declares a test anywhere inside it, as Ruby would run
/// it with the class body: a `test` call, a `def test_*`, or a
/// `define_method` whose name starts `test_`. A nested class or module
/// body is its own.
fn contains_test(node: &Node<'_>) -> bool {
    struct Find(bool);
    impl<'pr> ruby_prism::Visit<'pr> for Find {
        fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
            let name = constant_id_str(&node.name());
            let test_name = || {
                node.arguments().and_then(|a| a.arguments().iter().next()).is_some_and(|arg| {
                    // A name it cannot read (`define_method(name)`,
                    // `"#{prefix}_x"`) may be a test's, so it counts.
                    let whole = |t: &[u8]| t.starts_with(b"test_");
                    // An interpolation's leading text may stop short of `test_`.
                    let lead = |t: &[u8]| whole(t) || b"test_".starts_with(t);
                    let parts = if let Some(s) = arg.as_symbol_node() {
                        return whole(s.unescaped());
                    } else if let Some(s) = arg.as_string_node() {
                        return whole(s.unescaped());
                    } else if let Some(s) = arg.as_interpolated_string_node() {
                        s.parts()
                    } else if let Some(s) = arg.as_interpolated_symbol_node() {
                        s.parts()
                    } else {
                        return true;
                    };
                    parts.iter().next().and_then(|p| p.as_string_node()).is_none_or(|s| lead(s.unescaped()))
                })
            };
            // Rails' `test` takes a name and a block (a blockless one
            // flunks); `Kernel#test(?e, path)` takes two or more and none.
            let declares = node.block().is_some() || node.arguments().is_some_and(|a| a.arguments().iter().count() == 1);
            if node.receiver().is_none()
                && ((name == "test" && declares) || (name == "define_method" && test_name()))
            {
                self.0 = true;
            }
            ruby_prism::visit_call_node(self, node);
        }
        fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
            if node.receiver().is_none() && constant_id_str(&node.name()).starts_with("test_") {
                self.0 = true;
            }
        }
        // `class << self; def test_order` is the class's own method,
        // as a nested class or module body is its own.
        fn visit_singleton_class_node(&mut self, _: &ruby_prism::SingletonClassNode<'pr>) {}
        fn visit_class_node(&mut self, _: &ruby_prism::ClassNode<'pr>) {}
        fn visit_module_node(&mut self, _: &ruby_prism::ModuleNode<'pr>) {}
    }
    let mut find = Find(false);
    ruby_prism::Visit::visit(&mut find, node);
    find.0
}

/// The tests a class-body loop declares, one per element:
///
/// ```ruby
/// { "short" => "Hi", "long" => "Hello" }.each do |label, title|
///   test "a #{label} title is kept" do
///     assert_equal title, Article.new(title: title).title
///   end
/// end
/// ```
///
/// Minitest runs the loop when the class body does, so each `test`
/// call defines one method per element, named by its interpolated
/// string. Over a literal Array (or Array of Arrays, destructured) or
/// a literal Hash of plain literals, the names are known now: one test
/// per element, its body the block's with each loop parameter it reads
/// bound to the element's literal first, the value its closure saw.
///
/// `Ok(None)` for a statement that is no loop declaring tests. A loop
/// that declares them in any other shape is an error naming the file,
/// never a silent drop of its tests.
fn ingest_test_loop(stmt: &Node<'_>, file: &str) -> IngestResult<Option<Vec<Test>>> {
    let Some(call) = stmt.as_call_node() else { return Ok(None) };
    let Some(block) = call.block().and_then(|b| b.as_block_node()) else { return Ok(None) };
    let statements = block.body().map(flatten_statements).unwrap_or_default();
    if !block.body().is_some_and(|b| contains_test(&b)) {
        return Ok(None);
    }
    let method = constant_id_str(&call.name()).to_string();
    let refuse = |why: &str| IngestError::Unsupported {
        file: file.into(),
        message: format!(
            "tests declared inside a class-body `{method}` block are not emitted: {why}; \
             only `each` over a literal Array or Hash of plain literals is unrolled"
        ),
    };
    let Some(recv) = call.receiver().filter(|_| method == "each" && call.arguments().is_none()) else {
        return Err(refuse("the loop is not `each` over a literal"));
    };
    let params: Vec<String> = match block.parameters().map(|p| p.as_block_parameters_node()) {
        None => Vec::new(),
        Some(Some(p)) if p.locals().iter().next().is_none() => match p.parameters() {
            None => Vec::new(),
            Some(list)
                if list.optionals().iter().next().is_none()
                    && list.rest().is_none()
                    && list.posts().iter().next().is_none()
                    && list.keywords().iter().next().is_none()
                    && list.keyword_rest().is_none()
                    && list.block().is_none() =>
            {
                let names: Option<Vec<String>> = list
                    .requireds()
                    .iter()
                    .map(|r| Some(constant_id_str(&r.as_required_parameter_node()?.name()).to_string()))
                    .collect();
                names.ok_or_else(|| refuse("its block parameters are not plain names"))?
            }
            Some(_) => return Err(refuse("its block parameters are not plain names")),
        },
        _ => return Err(refuse("its block parameters are not plain names")),
    };
    // Each element as the values its parameters read, in order.
    let rows: Option<Vec<Vec<Node<'_>>>> = if let Some(array) = recv.as_array_node() {
        array
            .elements()
            .iter()
            .map(|e| match params.len() {
                // Nothing reads the element, but Ruby still evaluates it.
                0 => literal_text(&e).map(|_| Vec::new()),
                1 => Some(vec![e]),
                n => e.as_array_node().map(|a| a.elements().iter().collect::<Vec<_>>()).filter(|v| v.len() == n),
            })
            .collect()
    } else if let Some(hash) = recv.as_hash_node() {
        let rows: Option<Vec<Vec<Node<'_>>>> = (params.len() == 2)
            .then(|| {
                hash.elements()
                    .iter()
                    .map(|e| e.as_assoc_node().map(|a| vec![a.key(), a.value()]))
                    .collect()
            })
            .flatten();
        // `{ "x" => 1, "x" => 2 }` iterates one entry, the last value.
        let kind = |n: &Node<'_>| {
            [n.as_string_node().is_some(), n.as_symbol_node().is_some(), n.as_integer_node().is_some()]
                .iter()
                .position(|is| *is)
        };
        let keys: Vec<(Option<usize>, Option<String>)> =
            rows.iter().flatten().map(|row| (kind(&row[0]), literal_text(&row[0]))).collect();
        if keys.iter().enumerate().any(|(i, k)| k.1.is_some() && keys[..i].contains(k)) {
            return Err(refuse("its Hash repeats a key"));
        }
        rows
    } else {
        None
    };
    let rows = rows.ok_or_else(|| refuse("it does not iterate a literal its parameters destructure"))?;
    let texts: Vec<Vec<String>> = rows
        .iter()
        .map(|row| row.iter().map(literal_text).collect::<Option<Vec<_>>>())
        .collect::<Option<_>>()
        .ok_or_else(|| refuse("an element is not a plain literal"))?;
    let param_refs: Vec<&str> = params.iter().map(String::as_str).collect();
    let mut tests = Vec::new();
    for (row, text) in rows.iter().zip(&texts) {
        for s in &statements {
            let Some(test) = s.as_call_node().filter(|c| declares_test(s) && c.arguments().is_some()) else {
                return Err(refuse("its block holds more than `test` declarations"));
            };
            // Only the name: Ruby would evaluate any further argument.
            let args: Vec<Node<'_>> = test.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
            let [name_node] = args.as_slice() else {
                return Err(refuse("a test takes other than one name argument"));
            };
            let name = test_name(&name_node, &param_refs, row, text).ok_or_else(|| refuse("a test name is not a string the loop's parameters complete"))?;
            // A blockless `test` is Rails' `flunk`; a forwarded block
            // (`&body`) is not one this reads.
            let Some(body) = test.block().and_then(|b| b.as_block_node()) else {
                return Err(refuse("a test has no literal block"));
            };
            if body.parameters().is_some() {
                return Err(refuse("a test block takes parameters"));
            }
            let body = body.body();
            if body.as_ref().is_some_and(|b| !super::define_method_loop::closure_reads_only(b, &param_refs)) {
                return Err(refuse("a test body writes a loop parameter or reaches another outer local"));
            }
            let mut exprs = Vec::new();
            if let Some(body) = &body {
                let reads = local_reads(body);
                for (param, value) in params.iter().zip(row) {
                    if reads.contains(param) {
                        let name = Symbol::from(param.as_str());
                        exprs.push(Expr::new(
                            Span::synthetic(),
                            ExprNode::Assign { target: LValue::Var { id: VarId(0), name }, value: ingest_expr(value, file)? },
                        ));
                    }
                }
                exprs.push(ingest_expr(body, file)?);
            }
            // Rails refuses a second test of one method name at load.
            if tests.iter().any(|t: &Test| test_method_name(&t.name) == test_method_name(&name)) {
                return Err(refuse(&format!("two elements name the same test `{name}`")));
            }
            tests.push(Test { name, body: Expr::new(Span::synthetic(), ExprNode::Seq { exprs }) });
        }
    }
    Ok(Some(tests))
}

/// The method Rails' `test "name"` defines: `test_#{name.gsub(/\s+/, "_")}`.
fn test_method_name(name: &str) -> String {
    let mut out = String::from("test_");
    let mut in_space = false;
    for c in name.chars() {
        // Ruby's `\s`.
        if matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r') {
            if !in_space {
                out.push('_');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// Add a test the class body declares as Ruby method `method`. A second
/// `def` of one method replaces the first, as Ruby does; a second `test`
/// of one is refused, as Rails raises at load. Two different methods the
/// emit would name alike (`"YouTube"` and `"youtube"` are both
/// `test_youtube` there) are refused too: one would replace the other.
fn add_test(acc: &mut ClassBody, test: Test, method: String, redefine: bool, file: &str) -> IngestResult<()> {
    if let Some(at) = acc.test_methods.iter().position(|m| *m == method) {
        if redefine {
            acc.tests[at] = test;
            return Ok(());
        }
        return Err(IngestError::Unsupported {
            file: file.into(),
            message: format!("`{method}` is declared twice, which Rails refuses at load"),
        });
    }
    let emitted = crate::lower::test_module_to_library::sanitize_test_name(&test.name);
    if let Some(other) = acc.tests.iter().find(|t| crate::lower::test_module_to_library::sanitize_test_name(&t.name) == emitted) {
        return Err(IngestError::Unsupported {
            file: file.into(),
            message: format!(
                "the tests `{}` and `{}` would both be emitted as `test_{emitted}`, so one would replace the other",
                other.name, test.name
            ),
        });
    }
    acc.tests.push(test);
    acc.test_methods.push(method);
    Ok(())
}

/// What `"#{node}"` reads for a plain literal, or `None` for anything
/// else (a Float's text is Ruby's to decide, not its source's).
fn literal_text(node: &Node<'_>) -> Option<String> {
    if let Some(s) = node.as_string_node() {
        return Some(String::from_utf8_lossy(s.unescaped()).into_owned());
    }
    if let Some(s) = node.as_symbol_node() {
        return Some(String::from_utf8_lossy(s.unescaped()).into_owned());
    }
    if let Some(i) = node.as_integer_node() {
        let text = String::from_utf8_lossy(i.location().as_slice()).replace('_', "");
        // `010` (and `-010`) is octal in Ruby; only plain decimal text
        // reads as written.
        let digits = text.trim_start_matches(['-', '+']);
        if digits.len() > 1 && digits.starts_with('0') {
            return None;
        }
        return text.parse::<i64>().ok().map(|n| n.to_string());
    }
    if node.as_true_node().is_some() {
        return Some("true".into());
    }
    if node.as_false_node().is_some() {
        return Some("false".into());
    }
    node.as_nil_node().map(|_| String::new())
}

/// A loop test's name: a plain string, a parameter holding a String, or
/// a string interpolating only the loop's parameters, read with this
/// element's `texts`.
fn test_name(node: &Node<'_>, params: &[&str], row: &[Node<'_>], texts: &[String]) -> Option<String> {
    if let Some(s) = node.as_string_node() {
        return Some(String::from_utf8_lossy(s.unescaped()).into_owned());
    }
    if let Some(read) = node.as_local_variable_read_node().filter(|r| r.depth() == 0) {
        let at = params.iter().position(|p| *p == constant_id_str(&read.name()))?;
        // `test kind` names the test by the String itself; Rails'
        // `name.gsub` raises for anything else.
        return row[at].as_string_node().map(|_| texts[at].clone());
    }
    let mut out = String::new();
    for part in node.as_interpolated_string_node()?.parts().iter() {
        if let Some(s) = part.as_string_node() {
            out.push_str(&String::from_utf8_lossy(s.unescaped()));
            continue;
        }
        let stmts: Vec<Node<'_>> = part.as_embedded_statements_node()?.statements()?.body().iter().collect();
        let [only] = stmts.as_slice() else { return None };
        let read = only.as_local_variable_read_node().filter(|r| r.depth() == 0)?;
        let at = params.iter().position(|p| *p == constant_id_str(&read.name()))?;
        out.push_str(&texts[at]);
    }
    Some(out)
}

/// The names of the locals `node` reads, at any depth.
fn local_reads(node: &Node<'_>) -> Vec<String> {
    struct Reads(Vec<String>);
    impl<'pr> ruby_prism::Visit<'pr> for Reads {
        fn visit_local_variable_read_node(&mut self, node: &ruby_prism::LocalVariableReadNode<'pr>) {
            self.0.push(constant_id_str(&node.name()).to_string());
        }
    }
    let mut reads = Reads(Vec::new());
    ruby_prism::Visit::visit(&mut reads, node);
    reads.0
}

#[cfg(test)]
mod tests {
    use super::*;

    // campfire's `test/channels/room_messages_channel_test.rb`: two
    // `< ActionCable::Channel::TestCase` classes in one file, the
    // second naming its subject with `tests Turbo::StreamsChannel`.
    // Before `ingest_test_files`, the second class was demoted to a
    // helper and then skipped for being a test — two tests in no
    // tally.
    const TWO_CLASSES: &str = r#"
require "test_helper"

class RoomMessagesChannelTest < ActionCable::Channel::TestCase
  tests RoomMessagesChannel

  setup do
    @room = rooms(:designers)
  end

  test "a member may subscribe" do
    assert true
  end

  test "an outsider may not" do
    assert true
  end
end

class RoomMessagesViaStockTurboChannelTest < ActionCable::Channel::TestCase
  tests Turbo::StreamsChannel

  test "the stock channel refuses a room message stream" do
    assert true
  end
end
"#;

    #[test]
    fn every_test_class_in_a_file_is_its_own_module() {
        let tms = ingest_test_files(TWO_CLASSES.as_bytes(), "room_messages_channel_test.rb")
            .expect("ingest");
        let names: Vec<&str> = tms.iter().map(|tm| tm.name.0.as_str()).collect();
        assert_eq!(
            names,
            ["RoomMessagesChannelTest", "RoomMessagesViaStockTurboChannelTest"]
        );
        assert_eq!(tms[0].tests.len(), 2);
        assert_eq!(tms[1].tests.len(), 1);
        // Neither is the other's helper.
        assert!(tms.iter().all(|tm| tm.inner_classes.is_empty()));
        // The first module still carries its own setup.
        assert!(tms[0].setup.is_some());
        assert!(tms[1].setup.is_none());
    }

    #[test]
    fn tests_macro_names_the_subject() {
        let tms = ingest_test_files(TWO_CLASSES.as_bytes(), "room_messages_channel_test.rb")
            .expect("ingest");
        let targets: Vec<&str> = tms
            .iter()
            .map(|tm| tm.target.as_ref().map(|t| t.0.as_str()).unwrap_or(""))
            .collect();
        assert_eq!(targets, ["RoomMessagesChannel", "Turbo::StreamsChannel"]);
    }

    #[test]
    fn single_class_wrapper_still_answers_the_first() {
        let tm = ingest_test_file(TWO_CLASSES.as_bytes(), "room_messages_channel_test.rb")
            .expect("ingest")
            .expect("a test class");
        assert_eq!(tm.name.0.as_str(), "RoomMessagesChannelTest");
    }

    // A test class that a `module` wraps is the same class as
    // `class Models::PostTest`. Ingest used to read only the file's
    // top-level classes, so it lost this one and reported nothing.
    #[test]
    fn a_test_class_inside_a_module_keeps_its_namespace() {
        let src = r#"
module Models
  class Helper
  end

  class PostTest < ActiveSupport::TestCase
    class Inner
    end

    test "truth" do
      assert true
    end
  end
end
"#;
        let tms = ingest_test_files(src.as_bytes(), "post_test.rb").expect("ingest");
        let names: Vec<&str> = tms.iter().map(|tm| tm.name.0.as_str()).collect();
        assert_eq!(names, ["Models::PostTest"]);
        assert_eq!(tms[0].target.as_ref().map(|t| t.0.as_str()), Some("Post"));
        assert_eq!(tms[0].tests.len(), 1);
        let helpers: Vec<&str> =
            tms[0].inner_classes.iter().map(|lc| lc.name.0.as_str()).collect();
        assert_eq!(helpers, ["Inner", "Models::Helper"]);
    }

    // Ingest reads every `.rb` file in the test directories, so a
    // support class in a module reaches it too. It is not a test.
    #[test]
    fn a_support_class_inside_a_module_is_not_a_test() {
        let src = "module Support\n  class Thing\n    def x\n      1\n    end\n  end\nend\n";
        let tms = ingest_test_files(src.as_bytes(), "thing.rb").expect("ingest");
        assert!(tms.is_empty());
    }

    // When no class looks like a test, the first top-level class is
    // the test. A class inside a module does not take its place.
    #[test]
    fn the_fallback_test_class_is_a_top_level_class() {
        let src = r#"
module Fakes
  class Client
  end
end

class ProbeChecks < Minitest::Unit
  def test_x
    assert true
  end
end
"#;
        let tms = ingest_test_files(src.as_bytes(), "probe_checks.rb").expect("ingest");
        let names: Vec<&str> = tms.iter().map(|tm| tm.name.0.as_str()).collect();
        assert_eq!(names, ["ProbeChecks"]);
        let helpers: Vec<&str> =
            tms[0].inner_classes.iter().map(|lc| lc.name.0.as_str()).collect();
        assert_eq!(helpers, ["Fakes::Client"]);
    }

    fn loop_test_names(src: &str) -> IngestResult<Vec<String>> {
        let tms = ingest_test_files(src.as_bytes(), "loop_test.rb")?;
        Ok(tms[0].tests.iter().map(|t| t.name.clone()).collect())
    }

    // A test per element, as Minitest defines them when the class body
    // runs the loop. They used to be skipped with the loop.
    #[test]
    fn a_class_body_loop_declares_a_test_per_element() {
        let src = r##"
class LoopTest < ActiveSupport::TestCase
  { "short" => "Hi", "long" => "Hello" }.each do |label, title|
    test "a #{label} title" do
      assert_equal title, title.dup
    end
  end

  [[:one, 1], [:two, 2]].each do |name, n|
    test "#{name} is #{n}" do
      assert n
    end
  end

  %w[a b].each do |letter|
    test "letter #{letter}" do
    end
  end
end
"##;
        let names = loop_test_names(src).expect("ingest");
        assert_eq!(names, ["a short title", "a long title", "one is 1", "two is 2", "letter a", "letter b"]);
    }

    // Rails names `def test_a__b` itself and `test "a b"` `test_a_b`:
    // two tests, and a second `def` only redefines the first.
    #[test]
    fn test_methods_keep_their_ruby_names() {
        let src = "class NamesTest < ActiveSupport::TestCase\n  def test_a__b\n    assert false\n  end\n  def test_a__b\n  end\n  test \"a b c\" do\n  end\nend\n";
        assert_eq!(loop_test_names(src).expect("ingest"), ["a  b", "a b c"]);
    }

    // Neither is a test: Minitest's `self.test_order`, and `Kernel#test`.
    #[test]
    fn methods_named_like_tests_are_not_refused() {
        let src = "class OrderTest < ActiveSupport::TestCase\n  class << self\n    def test_order\n      :alphabetical\n    end\n  end\n  READY = test(?e, __FILE__)\n  [1].each { |n| test(?d, n.to_s) }\n  define_method(:t) { 1 }\n  def self.test_order\n    :alpha\n  end\n\n  test \"runs\" do\n  end\n  test \"order\" do\n  end\n  { 1 => \"one\", \"1\" => \"string one\" }.each do |k, v|\n    test v do\n    end\n  end\nend\n";
        assert_eq!(loop_test_names(src).expect("ingest"), ["runs", "order", "one", "string one"]);
    }

    // A loop it cannot unroll is an error naming the file, not a drop.
    #[test]
    fn a_loop_it_cannot_unroll_is_reported() {
        for (shape, why) in [
            ("KINDS.each do |kind|\n    test \"#{kind}\" do\n    end\n  end", "does not iterate a literal"),
            ("2.times do |n|\n    test \"#{n}\" do\n    end\n  end", "not `each` over a literal"),
            ("%w[a b].each do |kind|\n    test \"same\" do\n    end\n  end", "two elements name the same test"),
            ("%w[a b].each do |kind|\n    test \"#{kind.upcase}\" do\n    end\n  end", "not a string the loop's parameters complete"),
            ("test \"#{KIND} works\" do\n  end", "not a string literal"),
            ("[\"a b\", \"a  b\"].each do |kind|\n    test kind do\n    end\n  end", "two elements name the same test"),
            ("[010].each do |n|\n    test \"n#{n}\" do\n    end\n  end", "not a plain literal"),
            ("test \"one\" do\n  end\n  %w[one].each do |n|\n    test n do\n    end\n  end", "declared twice"),
            ("[prepare].each do\n    test \"ready\" do\n    end\n  end", "does not iterate a literal"),
            ("%w[a].each do |n|\n    test(n, &BODY)\n    test \"#{n} 2\" do\n    end\n  end", "no literal block"),
            ("%w[a].each do |n|\n    test \"#{n} pending\"\n    test \"#{n}\" do\n    end\n  end", "no literal block"),
            ("%w[a b].each do |n|\n    test \"#{n} pending\"\n  end", "no literal block"),
            ("{ \"x\" => 1, \"x\" => 2 }.each do |k, v|\n    test \"#{k} #{v}\" do\n    end\n  end", "repeats a key"),
            ("[:test_generated].each do |m|\n    define_method(m) do\n    end\n  end", "holds more than `test` declarations"),
            ("%w[YouTube youtube].each do |n|\n    test n do\n    end\n  end", "would both be emitted as `test_youtube`"),
            ("[1].each do |n|\n    test \"#{n}\", (raise \"bad\") do\n    end\n  end", "other than one name argument"),
            // One Ruby method; then two Ruby methods the emit names alike.
            ("def test_a__b\n  end\n  test \"a__b\" do\n  end", "`test_a__b` is declared twice"),
            ("def test_a__b\n  end\n  test \"a b\" do\n  end", "would both be emitted as `test_a_b`"),
            ("%w[one].each do |n|\n    test n do\n    end\n  end\n  test \"one\" do\n  end", "declared twice"),
            ("[-010].each do |n|\n    test \"n#{n}\" do\n    end\n  end", "not a plain literal"),
            ("%w[a b].each do |x|\n    %w[c d].each do |y|\n      test \"#{x}#{y}\" do\n      end\n    end\n  end", "holds more than `test` declarations"),
            ("%w[a b].each do |x|\n    if x\n      test x do\n      end\n    end\n  end", "holds more than `test` declarations"),
            ("if ENV[\"SLOW\"]\n    test \"slow\" do\n    end\n  end", "inside this class-body statement"),
            ("define_method(\"test_dynamic\") do\n  end", "inside this class-body statement"),
        ] {
            let src = format!("class LoopTest < ActiveSupport::TestCase\n  {shape}\nend\n");
            let err = loop_test_names(&src).expect_err(shape);
            let IngestError::Unsupported { file, message } = err else { panic!("{err:?}") };
            assert_eq!(file, "loop_test.rb");
            assert!(message.contains(why), "{shape}: {message}");
        }
    }
}
