//! A call no method of its receiver's class answers, on a class whose chain defines `method_missing`
//! (Hashie's `Mash`: `options.form`, `info.last_name?`), written as the `method_missing(:name, ...)` call Ruby makes
//! for it. Spinel dispatches no `method_missing` (every call is resolved as it builds) but calls one written out.
//! The Ruby family's trees only: there the rewrite is the call Ruby runs.

use crate::app::App;
use crate::det::HashMap;
use crate::expr::{Expr, ExprNode, Literal};
use crate::ident::{ClassId, Symbol};
use crate::ty::Ty;

pub fn apply_method_missing_calls(app: &mut App, registry: &HashMap<ClassId, crate::analyze::ClassInfo>, target: crate::build_target::BuildTarget) {
    if !target.has_runtime_relation() {
        return;
    }
    let missing = Symbol::from("method_missing");
    let answers_missing = |id: &ClassId| {
        crate::analyze::diagnostics::method_registered_on(registry, Some(&Ty::Class { id: id.clone(), args: vec![].into() }), &missing)
    };
    // The core container each class descends from, by its written parents: what its instances answer from there.
    let parents: HashMap<String, Option<String>> = app
        .library_classes
        .iter()
        .filter(|c| !c.is_module)
        .map(|c| (c.name.0.as_str().to_string(), c.parent.as_ref().map(|p| p.0.as_str().trim_start_matches("::").to_string())))
        .collect();
    // The class outside the program each class descends from, by its written parents (`ActiveSupport::OrderedOptions`
    // for `class Settings < ActiveSupport::OrderedOptions`).
    // The program's classes from `id` up, then the class outside it the chain ends at.
    let chain_of = |id: &ClassId| -> (Vec<String>, String) {
        let mut current = id.0.as_str().to_string();
        let mut chain = Vec::new();
        for _ in 0..32 {
            let Some(Some(parent)) = parents.get(&current) else { return (chain, current) };
            chain.push(current.clone());
            let scope: Vec<&str> = current.split("::").collect();
            let resolved = (0..scope.len())
                .rev()
                .map(|depth| if depth == 0 { parent.clone() } else { format!("{}::{parent}", scope[..depth].join("::")) })
                .find(|candidate| *candidate != current && parents.contains_key(candidate));
            match resolved {
                Some(next) => current = next,
                None => return (chain, parent.clone()),
            }
        }
        (chain, "Object".to_string())
    };
    let root_of = |id: &ClassId| chain_of(id).1;
    let own_update: crate::det::HashSet<String> = app
        .library_classes
        .iter()
        .filter(|c| c.methods.iter().any(|m| m.name.as_str() == "update" && m.receiver == crate::dialect::MethodReceiver::Instance))
        .map(|c| c.name.0.as_str().to_string())
        .collect();
    // Not a subclass's own `update`, which Ruby finds before the alias.
    let ordered_options = |id: &ClassId| {
        let (chain, root) = chain_of(id);
        root == "ActiveSupport::OrderedOptions" && !chain.iter().any(|c| own_update.contains(c))
    };
    let core_of = |id: &ClassId| -> &'static [&'static str] {
        // Not Object's alone for Rails' OrderedOptions, a Hash on the ruby family's tree (Spinel's holds one, and
        // answers only what its own class defines).
        match root_of(id).as_str() {
            "ActiveSupport::OrderedOptions" if target != crate::build_target::BuildTarget::Spinel => super::core_instance_methods::HASH,
            "Hash" => super::core_instance_methods::HASH,
            "Array" => super::core_instance_methods::ARRAY,
            "String" => super::core_instance_methods::STRING,
            _ => super::core_instance_methods::OBJECT,
        }
    };
    // Not `update` on Spinel's OrderedOptions, which defines only `merge!` (Rails' alias): an `update` beside it would type
    // openssl's `Digest#update` by its Hash (`runtime/spinel/ordered_options_spinel.rb`).
    if target == crate::build_target::BuildTarget::Spinel {
        fn rename(e: &mut Expr, ordered_options: &dyn Fn(&ClassId) -> bool) {
            e.node.for_each_child_mut(&mut |c| rename(c, ordered_options));
            // Not a class object (`PrefBox.update`, `self.class.update`): `Ty::Class` alone does not tell it from an instance.
            if let ExprNode::Send { recv: Some(r), method, .. } = &mut *e.node
                && method.as_str() == "update"
                && r.decisions & crate::expr::CLASS_OBJECT_VALUE == 0
                && !matches!(&*r.node, ExprNode::Const { .. })
                && !matches!(&*r.node, ExprNode::Send { method, args, .. } if method.as_str() == "class" && args.is_empty())
                && matches!(r.ty.as_ref().map(Ty::peel_nilable), Some(Ty::Class { id, .. }) if ordered_options(id))
            {
                *method = Symbol::from("merge!");
            }
        }
        crate::lower::for_each_emit_body(app, &mut |body| rename(body, &ordered_options));
    }
    // The classes whose nearest `method_missing` is private, which an explicit receiver reaches only through `__send__`.
    let private_missing: crate::det::HashSet<String> = {
        let own: HashMap<String, bool> = app
            .library_classes
            .iter()
            .flat_map(|c| c.methods.iter().filter(|m| m.name == missing && m.receiver == crate::dialect::MethodReceiver::Instance).map(move |m| (c.name.0.as_str().to_string(), m.visibility == crate::dialect::MethodVisibility::Private)))
            .collect();
        parents
            .keys()
            .filter(|name| {
                let mut current = Some((*name).clone());
                for _ in 0..32 {
                    let Some(c) = current else { break };
                    if let Some(private) = own.get(&c) {
                        return *private;
                    }
                    current = parents.get(&c).cloned().flatten();
                }
                false
            })
            .cloned()
            .collect()
    };
    // The core modules each class includes along its written chain (Warden's `Errors` takes `entries` from Enumerable).
    let includes: HashMap<String, Vec<String>> = app
        .library_classes
        .iter()
        .map(|c| (c.name.0.as_str().to_string(), c.includes.iter().map(|m| m.0.as_str().trim_start_matches("::").to_string()).collect()))
        .collect();
    let module_answers = |id: &ClassId, method: &str| {
        let mut current = Some(id.0.as_str().to_string());
        for _ in 0..32 {
            let Some(name) = current else { break };
            for m in includes.get(&name).into_iter().flatten() {
                let answers = match m.as_str() {
                    "Enumerable" => super::core_instance_methods::ENUMERABLE,
                    "Comparable" => super::core_instance_methods::COMPARABLE,
                    _ => &[][..],
                };
                if answers.contains(&method) {
                    return true;
                }
            }
            current = parents.get(&name).cloned().flatten();
        }
        false
    };
    // The instance methods each class's subclasses define: a receiverless call in the class reaches one of them on a
    // subclass's instance (view_component's `render_template_for` calls the `call` each component defines), which
    // Ruby dispatches before `method_missing`.
    // Not the parent as written: `class UI::Card < Base` names `UI::Base` when one is declared, as `core_of` resolves it.
    let qualified_parent = |name: &str| -> Option<String> {
        let parent = parents.get(name)?.clone()?;
        let scope: Vec<&str> = name.split("::").collect();
        (1..scope.len())
            .rev()
            .map(|depth| format!("{}::{parent}", scope[..depth].join("::")))
            .find(|candidate| candidate != name && parents.contains_key(candidate))
            .or(Some(parent))
    };
    let mut below: HashMap<String, crate::det::HashSet<String>> = HashMap::default();
    for lc in app.library_classes.iter().filter(|c| !c.is_module) {
        let names: Vec<String> = lc.methods.iter().filter(|m| m.receiver == crate::dialect::MethodReceiver::Instance).map(|m| m.name.as_str().to_string()).collect();
        let mut current = qualified_parent(lc.name.0.as_str());
        for _ in 0..32 {
            let Some(name) = current else { break };
            below.entry(name.clone()).or_default().extend(names.iter().cloned());
            current = qualified_parent(&name);
        }
    }
    for lc in &mut app.library_classes {
        let owner = lc.name.clone();
        let own_missing = answers_missing(&owner);
        let subclasses_define = below.get(owner.0.as_str()).cloned().unwrap_or_default();
        for m in lc.methods.iter_mut().filter(|m| m.receiver == crate::dialect::MethodReceiver::Instance) {
            // Not inside `method_missing` itself, where an unanswered name is the one it was asked.
            if m.name == missing || m.name.as_str() == "respond_to_missing?" {
                continue;
            }
            let mut locals: Vec<Symbol> = m.params.iter().map(|p| p.name.clone()).collect();
            // (a name a subclass defines is no miss: treated as the locals are, left as written)
            locals.extend(subclasses_define.iter().map(|n| Symbol::from(n.as_str())));
            rewrite(&mut m.body, &owner, own_missing, &locals, true, registry, &answers_missing, &core_of, &module_answers, &private_missing);
        }
    }
    // A block a class-body call hands over (`uid do request.params[options.uid_field] end`, replayed and instance_eval'd
    // on an instance): its receiverless reads are the instance's methods, typed by what the class answers for them.
    for lc in &mut app.library_classes {
        let owner = lc.name.clone();
        let Some(info) = registry.get(&owner) else { continue };
        for call in &mut lc.unknown_calls {
            let ExprNode::Send { block: Some(block), .. } = &mut *call.node else { continue };
            let ExprNode::Lambda { body, .. } = &mut *block.node else { continue };
            body_reads(body, info, &answers_missing, &core_of, &module_answers, registry, &private_missing);
        }
    }
    // Every other body reaches such a class through a receiver (`auth.provider` on OmniAuth's AuthHash in a model).
    let nobody = ClassId(Symbol::from(""));
    crate::lower::for_each_emit_body(app, &mut |body| rewrite(body, &nobody, false, &[], true, registry, &answers_missing, &core_of, &module_answers, &private_missing));
}

fn rewrite(
    e: &mut Expr,
    owner: &ClassId,
    own_missing: bool,
    locals: &[Symbol],
    typed: bool,
    registry: &HashMap<ClassId, crate::analyze::ClassInfo>,
    answers_missing: &dyn Fn(&ClassId) -> bool,
    core_of: &dyn Fn(&ClassId) -> &'static [&'static str],
    module_answers: &dyn Fn(&ClassId, &str) -> bool,
    private_missing: &crate::det::HashSet<String>,
) {
    e.node.for_each_child_mut(&mut |c| rewrite(c, owner, own_missing, locals, typed, registry, answers_missing, core_of, module_answers, private_missing));
    // Typed too where `typed` (the analysis answers a call no method takes with `method_missing`'s type), but not on a
    // receiver spelled as a class object (`Errors.new`, `self.class.new`): `Ty::Class` alone does not tell it from an instance.
    let instance_read = |r: &Expr| {
        r.decisions & crate::expr::CLASS_OBJECT_VALUE == 0
            && !matches!(&*r.node, ExprNode::Const { .. })
            && !matches!(&*r.node, ExprNode::Send { method, args, .. } if method.as_str() == "class" && args.is_empty())
    };
    let untyped = crate::analyze::diagnostics::is_unknown_ty(e.ty.as_ref());
    let ExprNode::Send { recv, method, args, parenthesized, .. } = &mut *e.node else { return };
    // (a receiverless call in an instance method is self's, an instance)
    if !untyped && !(typed && recv.as_ref().is_none_or(instance_read)) {
        return;
    }
    if method.as_str() == "method_missing" || method.as_str() == "new" || method.as_str().starts_with(|c: char| !c.is_ascii_alphabetic() && c != '_') {
        return;
    }
    let class = match recv.as_ref() {
        Some(r) => match r.ty.as_ref().map(Ty::peel_nilable) {
            Some(Ty::Class { id, .. }) if answers_missing(id) => id.clone(),
            _ => return,
        },
        None if own_missing && args.is_empty() && !locals.contains(method) => owner.clone(),
        None => return,
    };
    // Not a method the core class it descends from answers (`options.merge(...)` on a Hash subclass), a core module it
    // includes does, or Kernel's private one a receiverless call reaches (`block_given?`).
    if core_of(&class).contains(&method.as_str())
        || module_answers(&class, method.as_str())
        || (recv.is_none() && super::core_instance_methods::KERNEL_PRIVATE.contains(&method.as_str()))
    {
        return;
    }
    let private = recv.is_some() && private_missing.contains(class.0.as_str());
    let class_ty = Ty::Class { id: class, args: vec![].into() };
    if crate::analyze::diagnostics::method_registered_on(registry, Some(&class_ty), method) {
        return;
    }
    let name = Expr::new(e.span, ExprNode::Lit { value: Literal::Sym { value: method.clone() } });
    args.insert(0, name);
    *method = Symbol::from("method_missing");
    // Not a private `method_missing` called on its receiver, which raises: `__send__` reaches it as Ruby's dispatch does.
    if private {
        args.insert(0, Expr::new(e.span, ExprNode::Lit { value: Literal::Sym { value: Symbol::from("method_missing") } }));
        *method = Symbol::from("__send__");
    }
    // Not the source's bare `auth.provider`: an argument written without parens does not parse inside another call's.
    *parenthesized = true;
}

fn body_reads(
    e: &mut Expr,
    info: &crate::analyze::ClassInfo,
    answers_missing: &dyn Fn(&ClassId) -> bool,
    core_of: &dyn Fn(&ClassId) -> &'static [&'static str],
    module_answers: &dyn Fn(&ClassId, &str) -> bool,
    registry: &HashMap<ClassId, crate::analyze::ClassInfo>,
    private_missing: &crate::det::HashSet<String>,
) {
    e.node.for_each_child_mut(&mut |c| body_reads(c, info, answers_missing, core_of, module_answers, registry, private_missing));
    let ExprNode::Send { recv: Some(r), method, args, parenthesized, .. } = &mut *e.node else { return };
    let ExprNode::Send { recv: None, method: reader, args: reader_args, block: None, .. } = &*r.node else { return };
    if !reader_args.is_empty() || method.as_str() == "method_missing" || method.as_str() == "new" {
        return;
    }
    let held = match info.instance_methods.get(reader) {
        Some(Ty::Fn(fn_ty)) => (*fn_ty.ret).clone(),
        Some(t) => t.clone(),
        None => return,
    };
    let Ty::Class { id, .. } = held.peel_nilable() else { return };
    if !answers_missing(id) || core_of(id).contains(&method.as_str()) || module_answers(id, method.as_str()) {
        return;
    }
    if crate::analyze::diagnostics::method_registered_on(registry, Some(&Ty::Class { id: id.clone(), args: vec![].into() }), method) {
        return;
    }
    let name = Expr::new(e.span, ExprNode::Lit { value: Literal::Sym { value: method.clone() } });
    args.insert(0, name);
    *method = Symbol::from("method_missing");
    if private_missing.contains(id.0.as_str()) {
        args.insert(0, Expr::new(e.span, ExprNode::Lit { value: Literal::Sym { value: Symbol::from("method_missing") } }));
        *method = Symbol::from("__send__");
    }
    *parenthesized = true;
}
