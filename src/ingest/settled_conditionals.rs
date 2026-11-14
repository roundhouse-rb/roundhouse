// Not both arms kept: a `defined?` of a class the app carries is settled when the body loads, and Ruby never defines the other arm's methods (et-orbi's tzinfo 1.x fallbacks).
use crate::app::App;
use crate::dialect::{ConditionalDefinitions, LibraryClass, MethodDef};
use crate::expr::ExprNode;

pub(super) fn settle(app: &mut App) {
    let resolver = app.const_resolver.for_sources(&app.sources);
    let settler = Settler::of(app, &resolver);
    for lc in app.library_classes.iter_mut() {
        let records = std::mem::take(&mut lc.conditional_definitions);
        for record in records {
            match settler.arm(lc.name.0.as_str(), &record) {
                Some(kept) => drop_other_arms(lc, &record, kept),
                None => lc.conditional_definitions.push(record),
            }
        }
    }
}

/// What a load-time conditional settles to, as `settle` decides it, built
/// once over the program's classes. The concern splice asks it too, before
/// `settle` runs.
pub(super) struct Settler<'r> {
    known: crate::det::HashSet<String>,
    resolver: &'r crate::analyze::ConstResolver,
    class_side: ClassSide,
}

impl<'r> Settler<'r> {
    pub(super) fn of(app: &App, resolver: &'r crate::analyze::ConstResolver) -> Self {
        let known = app
            .library_classes
            .iter()
            .map(|c| c.name.0.as_str().to_string())
            .chain(app.models.iter().map(|m| m.name.0.as_str().to_string()))
            .collect();
        let mut settler = Settler { known, resolver, class_side: ClassSide::of(app) };
        // A class-side method in a conditional's arm answers a later
        // `respond_to?` only once that arm is known to run, and settling
        // one conditional can settle another: until no arm adds a method.
        loop {
            let mut reached = Vec::new();
            for lc in &app.library_classes {
                let owner = lc.name.0.as_str();
                for record in &lc.conditional_definitions {
                    let Some(arm) = settler.arm(owner, record).and_then(|k| record.arms.get(k)) else { continue };
                    reached.extend(
                        lc.methods
                            .iter()
                            .filter(|m| m.receiver == crate::dialect::MethodReceiver::Class && arm.methods.iter().any(|member| member.is(m)))
                            .map(|m| (owner, m)),
                    );
                }
            }
            let mut added = false;
            for (owner, m) in reached {
                added |= settler.class_side.add(owner, m);
            }
            if !added {
                break;
            }
        }
        settler
    }

    /// The arm of `record`, in the body of `owner`, that Ruby runs.
    fn arm(&self, owner: &str, record: &ConditionalDefinitions) -> Option<usize> {
        let here = Here { known: &self.known, resolver: self.resolver, class_side: &self.class_side, owner, span: record.span };
        settled_arm(record, &here)
    }

    /// Whether `method`, one of `lc`'s, is defined as Ruby loads the body:
    /// `Some(true)` outside every load-time conditional or in the arm one is
    /// known to take, `Some(false)` in an arm it is known to skip, `None`
    /// when only running the program answers.
    pub(super) fn defines(&self, lc: &LibraryClass, method: &MethodDef) -> Option<bool> {
        let Some(record) = lc.conditional_definitions.iter().find(|c| c.contains(method)) else { return Some(true) };
        let arm = record.arms.iter().position(|a| a.methods.iter().any(|m| m.is(method)))?;
        Some(self.arm(lc.name.0.as_str(), record)? == arm)
    }
}

/// The arm Ruby runs, or `usize::MAX` for none (an `if` without `else` whose condition is false).
/// Each class's class-side methods and superclass, for a class body's own `respond_to?`.
struct ClassSide {
    /// Each class-side method by name, with where its definition starts (a synthesized one has no file).
    methods: crate::det::HashMap<String, Vec<(String, crate::span::Span)>>,
    parents: crate::det::HashMap<String, String>,
}

impl ClassSide {
    /// The class-side methods defined outside every load-time conditional;
    /// one inside an arm is added when the arm is known to run.
    fn of(app: &App) -> Self {
        let mut methods: crate::det::HashMap<String, Vec<(String, crate::span::Span)>> = crate::det::HashMap::default();
        let mut parents = crate::det::HashMap::default();
        for lc in &app.library_classes {
            let name = lc.name.0.as_str().to_string();
            methods.entry(name.clone()).or_default().extend(
                lc.methods
                    .iter()
                    .filter(|m| m.receiver == crate::dialect::MethodReceiver::Class)
                    .filter(|m| !lc.conditional_definitions.iter().any(|c| c.contains(m)))
                    .map(|m| (m.name.as_str().to_string(), Self::start(m))),
            );
            if let Some(parent) = &lc.parent {
                parents.insert(name, parent.0.as_str().trim_start_matches("::").to_string());
            }
        }
        ClassSide { methods, parents }
    }

    /// Where a method's definition starts (a synthesized one has no file).
    fn start(m: &MethodDef) -> crate::span::Span {
        if m.name_span.is_synthetic() { m.body.span } else { m.name_span }
    }

    /// Adds `m` to `class`'s class-side methods; whether it was not there.
    fn add(&mut self, class: &str, m: &MethodDef) -> bool {
        let entry = (m.name.as_str().to_string(), Self::start(m));
        let methods = self.methods.entry(class.to_string()).or_default();
        if methods.contains(&entry) {
            return false;
        }
        methods.push(entry);
        true
    }

    /// Whether `class` or a superclass the program carries defines `name` on its class side by the time the body
    /// reaches `here`; `None` when none does, as a module it extends or a class outside the program may.
    fn responds(&self, class: &str, name: &str, here: crate::span::Span) -> Option<bool> {
        let mut at = Some(class.to_string());
        let mut seen = 0;
        while let Some(c) = at {
            let defined = self.methods.get(&c).into_iter().flatten().filter(|(m, _)| m == name);
            // Not any of the class's own: one defined further down its body, or in a reopening loaded who knows when, is not there yet.
            // A superclass's written further down the same file is not there yet either; one from another file is taken as loaded
            // before, as which of two files loads first is not known here.
            let same_file = |span: &crate::span::Span| !span.is_synthetic() && span.file == here.file;
            let before = |span: &crate::span::Span| {
                if c == class { same_file(span) && span.start < here.start } else { !same_file(span) || span.start < here.start }
            };
            if defined.into_iter().any(|(_, span)| before(span)) {
                return Some(true);
            }
            seen += 1;
            if seen > 64 {
                return None;
            }
            at = self.parents.get(&c).cloned();
        }
        None
    }
}

/// Where a conditional stands: what the program defines, and the class whose body holds it.
struct Here<'a> {
    known: &'a crate::det::HashSet<String>,
    resolver: &'a crate::analyze::ConstResolver,
    class_side: &'a ClassSide,
    owner: &'a str,
    span: crate::span::Span,
}

fn settled_arm(record: &ConditionalDefinitions, here: &Here<'_>) -> Option<usize> {
    if !matches!(record.arms.len(), 1 | 2)
        || record.arms.get(1).is_some_and(|a| a.condition.is_some())
        || record.arms.iter().any(|a| !a.nested.is_empty() || !a.declarations.is_empty())
    {
        return None;
    }
    let holds = settled(record.arms[0].condition.as_ref()?, here)? != record.negated;
    Some(match (holds, record.arms.len()) {
        (true, _) => 0,
        (false, 2) => 1,
        (false, _) => usize::MAX,
    })
}

/// A load-time condition whose answer the running Ruby (3.4 or later) and Spinel give alike.
fn settled(cond: &crate::expr::Expr, here: &Here<'_>) -> Option<bool> {
    let (known, resolver) = (here.known, here.resolver);
    match &*cond.node {
        ExprNode::Defined { operand } => {
            let ExprNode::Const { path } = &*operand.node else { return None };
            let name = path.iter().map(|p| p.as_str()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("::");
            // CRuby preloads these libraries; project assembly loads their
            // package on Spinel. Resolve the actual reference so a lexical
            // app constant with the same spelling does not select this arm.
            (known.contains(&name) || crate::gems::PRELOADED.iter().flat_map(|(_, _, classes)| *classes)
                .any(|class| resolver.is_runtime_class(operand.span, path, class))).then_some(true)
        }
        ExprNode::Send { recv: Some(r), method, args, .. } if method.as_str() == "!" && args.is_empty() => settled(r, here).map(|b| !b),
        ExprNode::Send { recv: None, method, args, .. } if method.as_str() == "!" && args.len() == 1 => settled(&args[0], here).map(|b| !b),
        ExprNode::BoolOp { op, left, right, .. } => {
            let l = settled(left, here);
            match op {
                crate::expr::BoolOpKind::Or if l == Some(true) => Some(true),
                crate::expr::BoolOpKind::And if l == Some(false) => Some(false),
                crate::expr::BoolOpKind::Or | crate::expr::BoolOpKind::And => {
                    let r = settled(right, here)?;
                    l.map(|_| r)
                }
            }
        }
        // The class body's own `respond_to?(:error_status=)`: the class answers for what it or a superclass defines.
        ExprNode::Send { recv, method, args, .. }
            if method.as_str() == "respond_to?" && recv.as_ref().is_none_or(|r| matches!(&*r.node, ExprNode::SelfRef)) =>
        {
            let [arg] = args.as_slice() else { return None };
            let ExprNode::Lit { value: crate::expr::Literal::Sym { value } } = &*arg.node else { return None };
            here.class_side.responds(here.owner, value.as_str(), here.span)
        }
        ExprNode::Send { recv: Some(r), method, args, .. } if method.as_str() == "respond_to?" => {
            let [arg] = args.as_slice() else { return None };
            let ExprNode::Lit { value: crate::expr::Literal::Sym { value } } = &*arg.node else { return None };
            let class = match &*r.node {
                ExprNode::Array { elements, .. } if elements.is_empty() => "Array",
                ExprNode::Send { recv: Some(c), method, args, .. }
                    if method.as_str() == "new" && args.is_empty() && matches!(&*c.node, ExprNode::Const { path } if path.last().is_some_and(|p| p.as_str() == "Object")) =>
                {
                    "Object"
                }
                _ => return None,
            };
            BUILTIN_ANSWERS.iter().find(|(c, m, _)| *c == class && *m == value.as_str()).map(|(_, _, has)| *has)
        }
        _ => None,
    }
}

/// What Ruby 3.4+ (and Spinel) answers to `respond_to?` for a builtin's method a gem probes for.
const BUILTIN_ANSWERS: &[(&str, &str, bool)] = &[
    ("Array", "bsearch", true),
    ("Array", "bsearch_index", true),
    ("Array", "sum", true),
    ("Array", "filter_map", true),
    // `taint` and its kin were removed in Ruby 3.2.
    ("Object", "untaint", false),
    ("Object", "taint", false),
    ("Object", "tainted?", false),
];

fn drop_other_arms(lc: &mut LibraryClass, record: &ConditionalDefinitions, kept: usize) {
    for (i, arm) in record.arms.iter().enumerate() {
        if i != kept {
            lc.methods.retain(|m| !arm.methods.iter().any(|member| member.is(m)));
        } else {
            lc.unknown_calls.extend(arm.statements.iter().cloned());
        }
    }
}
