//! A routing method a gem adds to the mapper, drawn from its own body: doorkeeper 5.9's
//! `use_doorkeeper do controllers … end` is `ActionDispatch::Routing::Mapper.include
//! Doorkeeper::Rails::Routes::Helper`, whose `use_doorkeeper` builds a router and calls
//! `routes.scope "oauth" do routes.resource … end` on the mapper it was given.
//!
//! Route ingest keeps each such call where it is written ([`RouteSpec::Deferred`]), since
//! the method reads options the class macros have not yet expanded. Once they have, the
//! method runs on the evaluator of `configured_superclass` with the mapper as a recording
//! value: each routing call on it is written out as route DSL, its block as the routes
//! nested in it. Those are read by the route ingest as any route file is, and stand in
//! the call's place, inside whatever scope encloses it. A call that does not run is
//! dropped and reported, as an unknown routing method is.

use std::path::Path;

use crate::App;
use crate::det::HashSet;
use crate::dialect::{LibraryClass, MethodDef, MethodReceiver, RouteSpec};
use crate::expr::{Expr, ExprNode, Literal};
use crate::vfs::Vfs;

/// The instance methods of the modules some statement of the program mixes into `ActionDispatch::Routing::Mapper`.
pub(super) fn mapper_methods(app: &App) -> HashSet<String> {
    extensions(app).into_iter().map(|(_, m)| m.name.as_str().to_string()).collect()
}

fn extensions(app: &App) -> Vec<(&LibraryClass, &MethodDef)> {
    let mut modules: Vec<String> = Vec::new();
    for lc in &app.library_classes {
        let bodies = lc.methods.iter().map(|m| &m.body).chain(lc.unknown_calls.iter());
        for body in bodies {
            visit(body, &mut |e| {
                let ExprNode::Send { recv: Some(r), method, args, .. } = &*e.node else { return };
                let ExprNode::Const { path } = &*r.node else { return };
                let written: Vec<&str> = path.iter().map(|s| s.as_str()).filter(|s| !s.is_empty()).collect();
                if written != ["ActionDispatch", "Routing", "Mapper"] {
                    return;
                }
                let mixed: Vec<&Expr> = match method.as_str() {
                    "include" | "prepend" => args.iter().collect(),
                    "send" | "__send__" if matches!(args.first().map(|a| &*a.node), Some(ExprNode::Lit { value: Literal::Sym { value } }) if matches!(value.as_str(), "include" | "prepend")) => args[1..].iter().collect(),
                    _ => return,
                };
                for arg in mixed {
                    if let ExprNode::Const { path } = &*arg.node {
                        let written = path.iter().map(|s| s.as_str()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("::");
                        modules.extend(resolve(app, &written, lc.name.0.as_str()));
                    }
                }
            });
        }
    }
    app.library_classes
        .iter()
        // The mapper reopened (`class ActionDispatch::Routing::Mapper; def devise_for`), as Devise writes it.
        .filter(|lc| (lc.is_module && modules.iter().any(|m| m == lc.name.0.as_str())) || lc.name.0.as_str() == "ActionDispatch::Routing::Mapper")
        .flat_map(|lc| lc.methods.iter().filter(|m| m.receiver == MethodReceiver::Instance).map(move |m| (lc, m)))
        .collect()
}

fn resolve(app: &App, written: &str, owner: &str) -> Option<String> {
    let mut scope = Some(owner.to_string());
    while let Some(outer) = scope {
        let candidate = format!("{outer}::{written}");
        if app.library_classes.iter().any(|c| c.name.0.as_str() == candidate) {
            return Some(candidate);
        }
        scope = outer.rsplit_once("::").map(|(o, _)| o.to_string());
    }
    app.library_classes.iter().any(|c| c.name.0.as_str() == written).then(|| written.to_string())
}

fn visit(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    e.node.for_each_child(&mut |c| visit(c, f));
}

/// Every deferred call in the route table: the routes its method draws, or nothing and a report.
pub(super) fn expand<V: Vfs + ?Sized>(vfs: &V, dir: &Path, app: &mut App) {
    let mut entries = std::mem::take(&mut app.routes.entries);
    let mut failed = false;
    if contains_deferred(&entries) {
        let initializers = super::configured_superclass::initializers_of(vfs, dir);
        failed = replace(&mut entries, app, &initializers);
    }
    app.routes.entries = entries;
    if !failed {
        consume_installs(app);
    }
}

/// `ActionDispatch::Routing::Mapper.include Helper`: its one effect, the mapper's new methods, is the
/// routes this pass drew, so the emitted program does not run it (it has no mapper to extend).
fn consume_installs(app: &mut App) {
    let modules: HashSet<String> = extensions(app).iter().map(|(lc, _)| lc.name.0.as_str().to_string()).collect();
    if modules.is_empty() {
        return;
    }
    for index in 0..app.library_classes.len() {
        let owner = app.library_classes[index].name.0.as_str().to_string();
        let mut lc = std::mem::replace(&mut app.library_classes[index], placeholder());
        for m in &mut lc.methods {
            rewrite(&mut m.body, &mut |e| install_of(e).is_some_and(|written| resolve(app, &written, &owner).is_some_and(|full| modules.contains(&full))));
        }
        app.library_classes[index] = lc;
    }
    // Not left in the emitted program: an extension's methods run only on the mapper, which drew its routes here (devise_for's body names OmniAuth).
    let included_elsewhere = |name: &str| {
        app.library_classes.iter().any(|lc| lc.name.0.as_str() != "ActionDispatch::Routing::Mapper" && lc.includes.iter().any(|i| i.0.as_str() == name))
    };
    let dead: HashSet<String> = modules.into_iter().filter(|m| !included_elsewhere(m)).collect();
    for lc in app.library_classes.iter_mut().filter(|lc| dead.contains(lc.name.0.as_str())) {
        lc.methods.retain(|m| m.receiver != MethodReceiver::Instance);
    }
}

fn placeholder() -> LibraryClass {
    LibraryClass {
        name: crate::ident::ClassId(crate::ident::Symbol::from("")),
        is_module: true,
        parent: None,
        parent_span: crate::span::Span::synthetic(),
        includes: Vec::new(),
        include_spans: Vec::new(),
        methods: Vec::new(),
        nullable_columns: Vec::new(),
        origin: None,
        constants: Vec::new(),
        conditional_definitions: Vec::new(),
        unknown_calls: Vec::new(),
        class_ivar_initializers: Vec::new(),
    }
}

/// The module a `ActionDispatch::Routing::Mapper.include M` statement names, as written.
fn install_of(e: &Expr) -> Option<String> {
    let ExprNode::Send { recv: Some(r), method, args, .. } = &*e.node else { return None };
    let ExprNode::Const { path } = &*r.node else { return None };
    let written: Vec<&str> = path.iter().map(|s| s.as_str()).filter(|s| !s.is_empty()).collect();
    if written != ["ActionDispatch", "Routing", "Mapper"] || !matches!(method.as_str(), "include" | "prepend") {
        return None;
    }
    let [arg] = args.as_slice() else { return None };
    let ExprNode::Const { path } = &*arg.node else { return None };
    Some(path.iter().map(|s| s.as_str()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("::"))
}

fn rewrite(e: &mut Expr, is_install: &mut dyn FnMut(&Expr) -> bool) {
    if is_install(e) {
        *e = Expr::new(e.span, ExprNode::Lit { value: Literal::Nil });
        return;
    }
    e.node.for_each_child_mut(&mut |c| rewrite(c, is_install));
}

fn contains_deferred(entries: &[RouteSpec]) -> bool {
    entries.iter().any(|e| match e {
        RouteSpec::Deferred { .. } => true,
        RouteSpec::Scope { entries, .. } => contains_deferred(entries),
        RouteSpec::Resources { nested, .. } => contains_deferred(nested),
        _ => false,
    })
}

fn replace(entries: &mut Vec<RouteSpec>, app: &App, initializers: &[Expr]) -> bool {
    let mut failed = false;
    for entry in entries.iter_mut() {
        match entry {
            RouteSpec::Scope { entries, .. } => failed |= replace(entries, app, initializers),
            RouteSpec::Resources { nested, .. } => failed |= replace(nested, app, initializers),
            RouteSpec::Deferred { call, span, constraints } => {
                let drawn = draw(app, call, initializers);
                *entry = match drawn {
                    Ok(mut inner) => {
                        super::routes::apply_outer_constraints(&mut inner, constraints);
                        RouteSpec::Scope {
                            path: None,
                            module: None,
                            as_prefix: None,
                            defaults: indexmap::IndexMap::new(),
                            nest: false,
                            suppress_helpers: false,
                            mount: false,
                            within: crate::dialect::ResourceScope::Nested,
                            entries: inner,
                        }
                    }
                    Err(why) => {
                        failed = true;
                        let method = match &*call.node {
                            ExprNode::Send { method, .. } => method.as_str().to_string(),
                            _ => "call".into(),
                        };
                        super::survey::record(&super::IngestError::Unsupported {
                            file: super::sources::path_of(span.file).unwrap_or_default(),
                            message: format!("unsupported routes DSL: `{method}` ({why})"),
                        });
                        RouteSpec::Scope {
                            path: None,
                            module: None,
                            as_prefix: None,
                            defaults: indexmap::IndexMap::new(),
                            nest: false,
                            suppress_helpers: false,
                            mount: false,
                            within: crate::dialect::ResourceScope::Nested,
                            entries: Vec::new(),
                        }
                    }
                };
            }
            _ => {}
        }
    }
    failed
}

fn draw(app: &App, call: &Expr, initializers: &[Expr]) -> Result<Vec<RouteSpec>, String> {
    let ExprNode::Send { method, .. } = &*call.node else { return Err("not a call".into()) };
    let (lc, def) = extensions(app).into_iter().find(|(_, m)| &m.name == method).ok_or("no method of a mapper extension")?;
    // The gems the method reaches: its module's, and each top-level constant the program defines that its body names (`Devise` from the reopened mapper).
    let mut roots: Vec<String> = vec![lc.name.0.as_str().split("::").next().unwrap_or_default().to_string()];
    visit(&def.body, &mut |e| {
        if let ExprNode::Const { path } = &*e.node
            && let Some(top) = path.iter().map(|s| s.as_str()).find(|s| !s.is_empty())
            && app.library_classes.iter().any(|c| c.name.0.as_str() == top)
            && !roots.iter().any(|r| r == top)
        {
            roots.push(top.to_string());
        }
    });
    let scope = def.enclosing_class.as_ref().map(|s| s.as_str().to_string()).unwrap_or_else(|| lc.name.0.as_str().to_string());
    let drawn = super::configured_superclass::draw_routes(app, def, scope, call, initializers, &roots)?;
    let body: String = drawn.iter().map(|e| crate::emit::ruby::emit_expr(e) + "\n").collect();
    let source = format!("Rails.application.routes.draw do\n{body}end\n");
    let table = super::routes::ingest_routes(source.as_bytes(), "<gem routes>").map_err(|e| format!("the drawn routes do not read: {e:?}"))?;
    Ok(table.entries)
}
