// Not left on the receiver: `permit`, `permit!`, `to_unsafe_h` and `require` are ActionController::Parameters methods, and a request's params reach the emitted program as plain String-keyed hashes.
use crate::app::App;
use crate::expr::{Expr, ExprNode, Literal};
use crate::ident::Symbol;
use crate::ty::Ty;

pub fn apply_params_permit_lowering(app: &mut App) {
    let lists = helper_filter_lists(&app.controllers);
    let none = HelperLists::default();
    super::for_each_owned_hook_body(app, &mut |owner, body| {
        rewrite(body, owner.and_then(|o| lists.get(o)).unwrap_or(&none))
    });
}

/// A controller's no-argument methods that answer a literal filter list, by name.
type HelperLists = crate::det::HashMap<Symbol, Vec<Expr>>;

/// Per controller, the methods it has or inherits whose body is a literal list of Symbols or Strings (`%i[a b]`,
/// `%w[a b]`, `[:a, "b"]`, optionally `.freeze`), so `permit(*helper)` can be read as the list written inline (as
/// `permit_splat` does for the typed params chain, #858). The nearest definition wins, and a list a descendant
/// overrides with another, or that may hide behind a module a descendant includes, is left out.
fn helper_filter_lists(
    controllers: &[crate::dialect::Controller],
) -> crate::det::HashMap<crate::ident::ClassId, HelperLists> {
    let by_name: crate::det::HashMap<&crate::ident::ClassId, &crate::dialect::Controller> =
        controllers.iter().map(|c| (&c.name, c)).collect();
    let mut out = crate::det::HashMap::default();
    for controller in controllers {
        let mut lists = HelperLists::default();
        let mut seen = crate::det::HashSet::default();
        let mut current = Some(controller);
        let mut depth = 0;
        while let Some(c) = current
            && depth < 32
        {
            // A later definition in one class body replaces an earlier one, as Ruby's does.
            for def in c.actions().collect::<Vec<_>>().into_iter().rev() {
                if !seen.insert(def.name.clone()) {
                    continue;
                }
                if def.formal_params().is_empty()
                    && def.block_param.is_none()
                    && let Some(list) = literal_filter_list(&def.body)
                {
                    lists.insert(def.name.clone(), list);
                }
            }
            depth += 1;
            current = c.parent.as_ref().and_then(|p| by_name.get(p).copied());
        }
        if !lists.is_empty() {
            out.insert(controller.name.clone(), lists);
        }
    }
    // An ancestor's body runs on its descendants too, where `*helper` answers the descendant's own list. A list a
    // descendant overrides with other names, or with a body that is not a literal list, is not the ancestor's to
    // write inline.
    let names_of = |list: &Vec<Expr>| filter_names(list);
    let overridden: Vec<(crate::ident::ClassId, Symbol)> = out
        .iter()
        .flat_map(|(owner, lists)| lists.iter().map(move |(name, list)| (owner, name, list)))
        .filter(|(owner, name, list)| {
            controllers.iter().any(|d| {
                d.name != **owner
                    && descends_from(d, owner, &by_name)
                    && (out.get(&d.name).and_then(|l| l.get(*name)).map(names_of) != Some(names_of(list))
                        || includes_unread_module(d))
            })
        })
        .map(|(owner, name, _)| (owner.clone(), name.clone()))
        .collect();
    for (owner, name) in overridden {
        if let Some(lists) = out.get_mut(&owner) {
            lists.remove(&name);
        }
    }
    out
}

/// Whether `controller` still includes or prepends a module after ingest spliced in the concerns it could read (a
/// gem's, or one the tree does not define): that module's methods are unknown here and may override a helper.
fn includes_unread_module(controller: &crate::dialect::Controller) -> bool {
    controller.body.iter().any(|item| {
        matches!(item, crate::dialect::ControllerBodyItem::Unknown { expr, .. }
            if matches!(&*expr.node, ExprNode::Send { recv: None, method, .. } if matches!(method.as_str(), "include" | "prepend")))
    })
}

/// Whether `ancestor` is on `controller`'s superclass chain.
fn descends_from(
    controller: &crate::dialect::Controller,
    ancestor: &crate::ident::ClassId,
    by_name: &crate::det::HashMap<&crate::ident::ClassId, &crate::dialect::Controller>,
) -> bool {
    let mut parent = controller.parent.as_ref();
    for _ in 0..32 {
        let Some(p) = parent else { return false };
        if p == ancestor {
            return true;
        }
        parent = by_name.get(p).and_then(|c| c.parent.as_ref());
    }
    false
}

/// The elements of a literal list of Symbols or Strings (`%i[a b]`, `["a", :b].freeze`), or `None`.
fn literal_filter_list(body: &Expr) -> Option<Vec<Expr>> {
    let body = match &*body.node {
        ExprNode::Seq { exprs } if exprs.len() == 1 => &exprs[0],
        _ => body,
    };
    let array = match &*body.node {
        ExprNode::Send { recv: Some(r), method, args, block: None, .. } if method.as_str() == "freeze" && args.is_empty() => r,
        _ => body,
    };
    let ExprNode::Array { elements, .. } = &*array.node else { return None };
    elements
        .iter()
        .all(|e| matches!(&*e.node, ExprNode::Lit { value: Literal::Sym { .. } | Literal::Str { .. } }))
        .then(|| elements.clone())
}

/// `permit`'s arguments with each splat of a literal list (`*%i[a b]`) or of a helper answering one
/// (`*shared_fields`) written inline, or `None` when a splat is neither.
fn inline_filter_splats(args: &[Expr], lists: &HelperLists) -> Option<Vec<Expr>> {
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        let ExprNode::Splat { value } = &*arg.node else {
            out.push(arg.clone());
            continue;
        };
        match &*value.node {
            ExprNode::Array { .. } => out.extend(literal_filter_list(value)?),
            ExprNode::Send { recv, method, args, block: None, .. }
                if args.is_empty() && recv.as_ref().is_none_or(|r| matches!(&*r.node, ExprNode::SelfRef)) =>
            {
                out.extend(lists.get(method)?.iter().cloned());
            }
            _ => return None,
        }
    }
    Some(out)
}

fn is_param_value(ty: Option<&Ty>) -> bool {
    matches!(ty, Some(Ty::Class { id, .. }) if id.0.as_str() == crate::analyze::PARAM_VALUE)
}

/// Rails' `ActionController::Parameters`, which a gem's code names (Devise's `ParameterSanitizer`), or a value read
/// out of one: on the emitted program both are the request's String-keyed Hash.
fn is_parameters(ty: Option<&Ty>) -> bool {
    match ty {
        Some(Ty::Class { id, .. }) => id.0.as_str() == "ActionController::Parameters" || id.0.as_str() == crate::analyze::PARAM_VALUE,
        Some(Ty::Union { variants }) => variants.iter().all(|v| *v == Ty::Nil || is_parameters(Some(v))),
        _ => false,
    }
}

/// Parameters beside another Hash (Devise's `parse_authentication_key_values(hash, keys)`, handed a request's
/// params and a Symbol-keyed Hash): which one a read meets is the run time's.
/// Parameters among other values: only Parameters answer `permit`.
fn holds_parameters(ty: Option<&Ty>) -> bool {
    matches!(ty, Some(Ty::Union { variants }) if variants.iter().any(|v| is_parameters(Some(v))))
}

fn may_be_parameters(ty: Option<&Ty>) -> bool {
    matches!(ty, Some(Ty::Union { variants }) if variants.iter().any(|v| is_parameters(Some(v)))
        && variants.iter().all(|v| is_parameters(Some(v)) || matches!(v, Ty::Hash { .. } | Ty::Nil)))
}

/// A request's own params, read through `params` (Warden's strategies' `request.params`), which Rails reads by a
/// Symbol as by its String. Not any `Hash[String, …]` a variable holds: a permitted Symbol-keyed Hash reaches those too.
fn is_request_params_read(e: &Expr) -> bool {
    matches!(&*e.node, ExprNode::Send { method, args, .. } if method.as_str() == "params" && args.is_empty())
        && matches!(e.ty.as_ref(), Some(Ty::Hash { key, value }) if **key == Ty::Str && is_param_value(Some(value)))
}

fn is_request_params(e: &Expr) -> bool {
    match &*e.node {
        ExprNode::Send { recv: None, method, args, .. } => method.as_str() == "params" && args.is_empty(),
        ExprNode::Ivar { name } => name.as_str() == "params",
        _ => false,
    }
}

/// Array methods whose block's first parameter is one element.
const ELEMENT_BLOCK_METHODS: &[&str] =
    &["map", "collect", "each", "flat_map", "filter_map", "select", "filter", "reject", "each_with_index"];

/// `params.require(:list)`, before or after it lowers to `Params.require_key`.
fn is_required_list(e: &Expr) -> bool {
    let ExprNode::Send { recv: Some(r), method, args, block: None, .. } = &*e.node else { return false };
    match method.as_str() {
        "require" => is_request_params(r) && args.len() == 1,
        "require_key" => {
            matches!(&*r.node, ExprNode::Const { path } if path.len() == 1 && path[0].as_str() == "Params")
                && args.first().is_some_and(is_request_params)
        }
        _ => false,
    }
}

/// `params.require(:list).map { |item| ... }`: Rails makes each element an
/// `ActionController::Parameters` (actionpack 8.1.4), and the runtime's
/// are String-keyed Hashes, so the element's own calls lower here.
fn rewrite_element_block(expr: &mut Expr, lists: &HelperLists) {
    let ExprNode::Send { recv: Some(r), method, block: Some(block), .. } = &mut *expr.node else { return };
    if !ELEMENT_BLOCK_METHODS.contains(&method.as_str()) || !is_required_list(r) {
        return;
    }
    let ExprNode::Lambda { params, body, .. } = &mut *block.node else { return };
    let Some(item) = params.first().cloned() else { return };
    // A block that reassigns the name holds another value there (`w = { kind: "old" }`): its keys are not the element's.
    if assigns_local(body, &item) {
        return;
    }
    rewrite_element_calls(body, &item, lists);
}

/// Whether `expr` binds the local `name` anywhere, after which it no longer holds the element. Every binding form
/// a source program can reach here with: an assignment (`=`, `op=`, `||=`, `&&=`, a multiple assignment), a pattern
/// binding (`=> w`, `in w`, a `case … in` arm) and a `rescue … => w`. `for w in …` and a named capture
/// (`/(?<w>…)/ =~ s`) arrive as plain assignments, as ingest desugars them. Not inside a nested block or lambda that
/// binds its own `name`: that write is to its parameter.
fn assigns_local(expr: &Expr, name: &Symbol) -> bool {
    if binds_name(expr, name) {
        return false;
    }
    let is_name = |lv: &crate::expr::LValue| matches!(lv, crate::expr::LValue::Var { name: n, .. } if n == name);
    let mut found = match &*expr.node {
        ExprNode::Assign { target, .. } | ExprNode::OpAssign { target, .. } => is_name(target),
        ExprNode::MultiAssign { targets, .. } => targets.iter().any(is_name),
        ExprNode::MatchRequired { pattern, .. } | ExprNode::MatchPredicate { pattern, .. } => pattern_binds(pattern, name),
        ExprNode::CaseMatch { arms, .. } => arms.iter().any(|arm| pattern_binds(&arm.pattern, name)),
        ExprNode::BeginRescue { rescues, .. } => rescues.iter().any(|r| r.binding.as_ref() == Some(name)),
        _ => false,
    };
    expr.node.for_each_child(&mut |c| found |= assigns_local(c, name));
    found
}

/// Whether a match pattern binds the local `name` anywhere in it (`w`, `{ kind: } => w`, `[*w]`, `{ w: }`, `**w`).
fn pattern_binds(pattern: &crate::expr::MatchPattern, name: &Symbol) -> bool {
    use crate::expr::{HashRest, MatchPattern};
    match pattern {
        MatchPattern::Value { .. } | MatchPattern::Nil => false,
        MatchPattern::Bind { name: n } => n == name,
        MatchPattern::Capture { pattern, name: n } => n == name || pattern_binds(pattern, name),
        MatchPattern::Alt { alternatives } => alternatives.iter().any(|p| pattern_binds(p, name)),
        MatchPattern::Array { pre, rest, post, .. } => {
            matches!(rest, Some(Some(n)) if n == name) || pre.iter().chain(post).any(|p| pattern_binds(p, name))
        }
        MatchPattern::Find { pre_rest, middle, post_rest, .. } => {
            pre_rest.as_ref() == Some(name) || post_rest.as_ref() == Some(name) || middle.iter().any(|p| pattern_binds(p, name))
        }
        MatchPattern::Hash { pairs, rest, .. } => {
            matches!(rest, Some(HashRest::Collect { name: n }) if n == name)
                || pairs.iter().any(|(key, p)| match p {
                    Some(p) => pattern_binds(p, name),
                    None => key == name,
                })
        }
    }
}

/// Whether `expr` is a block or lambda that binds `name` under any parameter kind (`|name|`, `name = {}`, `*name`,
/// `name:`, `&name`), so `name` inside it is its own value.
fn binds_name(expr: &Expr, name: &Symbol) -> bool {
    matches!(&*expr.node, ExprNode::Lambda { params, rest_param, extra_params, block_param, .. }
        if params.contains(name)
            || rest_param.as_ref() == Some(name)
            || block_param.as_ref() == Some(name)
            || extra_params.iter().any(|p| &p.name == name))
}

fn rewrite_element_calls(expr: &mut Expr, item: &Symbol, lists: &HelperLists) {
    // A nested block or lambda that binds the element's name reads its own value there, not the element.
    if binds_name(expr, item) {
        return;
    }
    expr.node.for_each_child_mut(&mut |c| rewrite_element_calls(c, item, lists));
    // A write by Symbol key (`item[:kind] = v`, `item[:kind] ||= v`) stores the String key the reads beside it read:
    // on Rails' Parameters both spellings are one key.
    if let ExprNode::Assign { target: crate::expr::LValue::Index { recv, index }, .. }
    | ExprNode::OpAssign { target: crate::expr::LValue::Index { recv, index }, .. } = &mut *expr.node
        && matches!(&*recv.node, ExprNode::Var { name, .. } if name == item)
    {
        if let Some(name) = sym_name(index) {
            *index = str_lit(index.span, name);
        }
        return;
    }
    // The same write as one target of a multiple assignment (`item[:kind], other = v, w`).
    if let ExprNode::MultiAssign { targets, .. } = &mut *expr.node {
        for target in targets {
            if let crate::expr::LValue::Index { recv, index } = target
                && matches!(&*recv.node, ExprNode::Var { name, .. } if name == item)
                && let Some(name) = sym_name(index)
            {
                *index = str_lit(index.span, name);
            }
        }
        return;
    }
    let ExprNode::Send { recv: Some(r), method, args, block, .. } = &mut *expr.node else { return };
    if !matches!(&*r.node, ExprNode::Var { name, .. } if name == item) {
        return;
    }
    match method.as_str() {
        "permit" | "expect" if block.is_none() => {
            // `permit(*shared_fields, :x)`: Rails flattens the filters, so a splat of a literal list is that list.
            let ExprNode::Send { args, .. } = &*expr.node else { return };
            let inlined = if args.iter().any(|a| matches!(&*a.node, ExprNode::Splat { .. })) {
                let Some(args) = inline_filter_splats(args, lists) else { return };
                let mut e = expr.clone();
                if let ExprNode::Send { args: slot, .. } = &mut *e.node {
                    *slot = args;
                }
                Some(e)
            } else {
                None
            };
            if let Some(mut new) = element_filter(inlined.as_ref().unwrap_or(expr)) {
                new.ty = expr.ty.clone();
                *expr = new;
            }
        }
        "[]" | "[]=" | "fetch" | "key?" | "has_key?" | "include?" | "member?" | "dig" => {
            let keys = if method.as_str() == "dig" { args.len() } else { args.len().min(1) };
            for key in args.iter_mut().take(keys) {
                if let Some(name) = sym_name(key) {
                    *key = str_lit(key.span, name);
                }
            }
        }
        _ => {}
    }
}

/// An element's `permit(:a, b: [...])`, and `expect(:a)`, which is Rails'
/// permit then require: a nested or blank value raises `ParameterMissing`.
fn element_filter(e: &Expr) -> Option<Expr> {
    let ExprNode::Send { recv: Some(r), method, args, .. } = &*e.node else { return None };
    let params_const = || Expr::new(e.span, ExprNode::Const { path: vec![Symbol::from("Params")] });
    let allowed = |names: Vec<String>| allowed_scalars(e.span, r, names);
    if method.as_str() == "expect" {
        let [key] = args.as_slice() else { return None };
        let name = sym_name(key)?;
        return Some(send(e.span, Some(params_const()), "require_key", vec![allowed(vec![name.clone()]), str_lit(e.span, name)]));
    }
    if matches!(args.last().map(|a| &*a.node), Some(ExprNode::Hash { .. })) {
        return filtered_call(e);
    }
    if args.is_empty() {
        return None;
    }
    Some(allowed(filter_names(args)?))
}

/// `params[:spec]&.dig(:arch)`, which ingest binds as `(__safe_nav = params[:spec]) && __safe_nav.dig(:arch)`: the
/// temporary holds the same params value `params[:spec].dig(:os)` reads, so its Symbol literal keys are the String
/// keys the request's Hash holds, as the emit writes them for the plain call.
fn stringify_safe_nav_param_reads(expr: &mut Expr) {
    let ExprNode::BoolOp { op: crate::expr::BoolOpKind::And, left, right, .. } = &mut *expr.node else { return };
    let ExprNode::Assign { target: crate::expr::LValue::Var { name: temp, .. }, value } = &*left.node else { return };
    // A value read out of the request's params, which keeps its String keys. Not any Parameters: a `permit` result
    // (`params.permit(:a)&.dig(:a)`) is lowered to a Symbol-keyed Hash, which its Symbol read finds.
    let params_value = is_param_value(value.ty.as_ref())
        || matches!(&*value.node, ExprNode::Send { recv: Some(r), method, .. } if method.as_str() == "[]" && is_request_params(r));
    if !temp.as_str().starts_with("__safe_nav") || !params_value {
        return;
    }
    let ExprNode::Send { recv: Some(r), method, args, .. } = &mut *right.node else { return };
    if !matches!(&*r.node, ExprNode::Var { name, .. } if name == temp) {
        return;
    }
    let keys = match method.as_str() {
        "dig" => args.len(),
        // Not `include?` / `member?`: a params value may be an Array, where they test an element, not a key.
        "[]" | "fetch" | "key?" | "has_key?" => args.len().min(1),
        _ => 0,
    };
    for key in args.iter_mut().take(keys) {
        if let Some(name) = sym_name(key) {
            *key = str_lit(key.span, name);
        }
    }
}

fn rewrite(expr: &mut Expr, lists: &HelperLists) {
    rewrite_element_block(expr, lists);
    expr.node.for_each_child_mut(&mut |c| rewrite(c, lists));
    stringify_safe_nav_param_reads(expr);
    // A top-level `params.permit(:a, :b)` CONSUMED IN PLACE, as a call's
    // argument — the Rails 8 authentication generator's
    // `@user.update(params.permit(:password, :password_confirmation))`
    // and `User.authenticate_by(params.permit(:email_address,
    // :password))`. One RETURNED from a params helper (`def note_params
    // = params.permit(:body)`) is the controller lowering's: it becomes
    // that controller's typed params class.
    if let ExprNode::Send { args, .. } = &mut *expr.node {
        for arg in args.iter_mut() {
            if let Some(permitted) = permitted_chain(arg) {
                *arg = permitted;
            }
        }
    }
    symbolize_permitted_key_args(expr, &crate::det::HashSet::default());
    if parameters_on_hash(expr) {
        return;
    }
    let ExprNode::Send { recv: Some(r), method, args, block: None, .. } = &*expr.node else { return };
    let nested = is_param_value(r.ty.as_ref());
    let replacement = match (method.as_str(), args.as_slice()) {
        ("permit!" | "to_unsafe_h", []) if nested || is_request_params(r) => Some(r.clone()),
        ("permit", keys) if nested && !keys.is_empty() => match filter_names(keys) {
            // The all-Symbol spelling, as main lowers it.
            Some(names) if keys.iter().all(|k| sym_name(k).is_some()) => {
                Some(send(expr.span, Some(r.clone()), "slice", names.into_iter().map(|n| str_lit(expr.span, n)).collect()))
            }
            // A String or Array-literal filter (`permit("a")`, `permit(%w[a b])`) keeps only scalar values, as Rails'
            // scalar filter does, with the scalar filter an element's `permit` lowers to: a Hash or an Array is dropped.
            Some(names) => Some(allowed_scalars(expr.span, r, names)),
            // maybe's `params.fetch(:q, {}).permit(:search, accounts: [])`: the nested filters a request's own permit takes.
            None => filtered_call(expr),
        },
        ("require", [key]) if nested => {
            let Some(name) = sym_name(key) else { return };
            let params = Expr::new(expr.span, ExprNode::Const { path: vec![Symbol::from("Params")] });
            Some(send(expr.span, Some(params), "require_key", vec![r.clone(), str_lit(expr.span, name)]))
        }
        _ => None,
    };
    if let Some(mut new) = replacement {
        new.ty = expr.ty.clone();
        *expr = new;
    }
}

/// Every top-level `params.permit(:a, :b)` left in a controller body, in
/// any position: a params helper's tail that names no resource
/// (lobsters' `def search_params = params.permit(:q, :what, …)`) is not
/// a typed params class's, and emitted verbatim it called `permit` on a
/// Hash.
pub(crate) fn rewrite_bare_permits(expr: &mut Expr) {
    expr.node.for_each_child_mut(&mut rewrite_bare_permits);
    if let Some(mut permitted) = permitted_chain(expr) {
        permitted.ty = Some(permitted_ty());
        *expr = permitted;
    } else if let Some(filtered) = filtered_chain(expr) {
        *expr = filtered;
    }
    symbolize_permitted_key_args(expr, &crate::det::HashSet::default());
}

/// Whether `e` answers the Symbol-keyed Hash a top-level `permit` lowers to (`Params.permitted`, or the `permit` call
/// still to be lowered), a no-argument call of one of `helpers` that answers it (`permitted_helpers`), or what a call
/// that keeps its keys answers from either (`params.permit(%w[a b]).merge(c: 1).compact`). By how the value was built,
/// not by its type: a `Hash[Symbol, String]` the app builds itself keeps Ruby's String-is-not-a-Symbol answers.
fn from_permitted(e: &Expr, helpers: &crate::det::HashSet<Symbol>) -> bool {
    if permitted_chain(e).is_some() || is_helper_call(e, helpers) {
        return true;
    }
    let ExprNode::Send { recv: Some(r), method, .. } = &*e.node else { return false };
    match method.as_str() {
        "permitted" => matches!(&*r.node, ExprNode::Const { path } if path.len() == 1 && path[0].as_str() == "Params"),
        "merge" | "reverse_merge" | "with_defaults" | "compact" | "compact_blank" | "except" | "slice" => {
            from_permitted(r, helpers)
        }
        _ => false,
    }
}

/// `x_params` or `self.x_params`, with no arguments and no block, naming one of `helpers`.
fn is_helper_call(e: &Expr, helpers: &crate::det::HashSet<Symbol>) -> bool {
    matches!(&*e.node, ExprNode::Send { recv, method, args, block: None, .. }
        if args.is_empty()
            && helpers.contains(method)
            && recv.as_ref().is_none_or(|r| matches!(&*r.node, ExprNode::SelfRef)))
}

/// The private helpers with no parameters whose value is the Symbol-keyed Hash a top-level `permit` with literal
/// names lowers to (`def x_params = params.permit(%w[a b])`, `def x_params = @x ||= params.permit(...)`), before or
/// after that lowering, so a key-taking call on their result names its Symbol keys (#2279).
pub(crate) fn permitted_helpers(privs: &[crate::dialect::Action]) -> crate::det::HashSet<Symbol> {
    let none = crate::det::HashSet::default();
    privs
        .iter()
        .filter(|a| a.params.fields.is_empty() && from_permitted(value_tail(&a.body), &none))
        .map(|a| a.name.clone())
        .collect()
}

/// What a body answers: its last statement, through an assignment's value (`@x ||= value`).
fn value_tail(e: &Expr) -> &Expr {
    match &*e.node {
        ExprNode::Seq { exprs } if !exprs.is_empty() => value_tail(exprs.last().unwrap()),
        ExprNode::OpAssign { value, .. } | ExprNode::Assign { value, .. } => value_tail(value),
        _ => e,
    }
}

/// `x_params.except("a")` → `x_params.except(:a)` for each of `helpers` (`permitted_helpers`), as
/// `symbolize_permitted_key_args` does on the `permit` written inline.
pub(crate) fn symbolize_permitted_helper_reads(expr: &mut Expr, helpers: &crate::det::HashSet<Symbol>) {
    if helpers.is_empty() {
        return;
    }
    expr.node.for_each_child_mut(&mut |c| symbolize_permitted_helper_reads(c, helpers));
    symbolize_permitted_key_args(expr, helpers);
}

/// `params.permit(%w[a b]).except("a")` → `.except(:a)`. Rails' Parameters take a String key as they take a Symbol,
/// and the Hash `permit` lowered to holds Symbol keys, so a String key name on it is its Symbol: a String literal
/// becomes the Symbol, `*%w[a]` becomes `*%i[a]`, another String `key.to_sym`, another splatted String Array
/// `*keys.map { |x| x.to_sym }`, and a String-keyed Hash literal `merge` takes gets Symbol keys (#2279).
fn symbolize_permitted_key_args(expr: &mut Expr, helpers: &crate::det::HashSet<Symbol>) {
    let ExprNode::Send { recv: Some(r), method, args, .. } = &mut *expr.node else { return };
    if !from_permitted(r, helpers) {
        return;
    }
    match method.as_str() {
        "except" | "slice" | "values_at" => args.iter_mut().for_each(symbolize_key),
        "[]" | "fetch" | "key?" | "has_key?" | "include?" | "member?" | "delete" | "dig" => {
            args.iter_mut().take(1).for_each(symbolize_key)
        }
        "merge" | "reverse_merge" | "with_defaults" => {
            for arg in args.iter_mut() {
                let ExprNode::Hash { entries, .. } = &mut *arg.node else { continue };
                for (key, _) in entries.iter_mut() {
                    symbolize_literal(key);
                }
                let all_symbols = entries.iter().all(|(k, _)| sym_name(k).is_some());
                if let Some(Ty::Hash { key, .. }) = &mut arg.ty
                    && all_symbols
                {
                    *key = std::sync::Arc::new(Ty::Sym);
                }
            }
        }
        _ => {}
    }
}

/// A String literal → its Symbol; false for anything else.
fn symbolize_literal(e: &mut Expr) -> bool {
    let ExprNode::Lit { value: Literal::Str { value } } = &*e.node else { return false };
    let sym = Symbol::from(value.as_str());
    *e.node = ExprNode::Lit { value: Literal::Sym { value: sym } };
    e.ty = Some(Ty::Sym);
    true
}

/// One key argument: a String literal, a splatted String Array literal, or another String (or splatted String Array).
fn symbolize_key(arg: &mut Expr) {
    if symbolize_literal(arg) {
        return;
    }
    let span = arg.span;
    let sym_array = || Some(Ty::Array { elem: std::sync::Arc::new(Ty::Sym) });
    if let ExprNode::Splat { value } = &mut *arg.node {
        if let ExprNode::Array { elements, style } = &mut *value.node {
            if !elements.is_empty() && elements.iter().all(|e| matches!(&*e.node, ExprNode::Lit { value: Literal::Str { .. } })) {
                for element in elements.iter_mut() {
                    symbolize_literal(element);
                }
                if *style == crate::expr::ArrayStyle::PercentW {
                    *style = crate::expr::ArrayStyle::PercentI;
                }
                value.ty = sym_array();
                arg.ty = Some(Ty::Sym);
            }
            return;
        }
        if !matches!(&value.ty, Some(Ty::Array { elem }) if **elem == Ty::Str) {
            return;
        }
        // `*keys` → `*keys.map { |x| x.to_sym }`.
        let x = Symbol::from("x");
        let mut var = Expr::new(span, ExprNode::Var { id: crate::ident::VarId(0), name: x.clone() });
        var.ty = Some(Ty::Str);
        let mut body = send(span, Some(var), "to_sym", vec![]);
        body.ty = Some(Ty::Sym);
        let block = Expr::new(span, ExprNode::Lambda {
            extra_params: Vec::new(),
            params: vec![x],
            rest_param: None,
            block_param: None,
            body,
            block_style: crate::expr::BlockStyle::Brace,
        });
        let mut mapped = Expr::new(span, ExprNode::Send {
            recv: Some(value.clone()),
            method: Symbol::from("map"),
            args: vec![],
            block: Some(block),
            parenthesized: false,
        });
        mapped.ty = sym_array();
        *value = mapped;
        arg.ty = Some(Ty::Sym);
        return;
    }
    if matches!(arg.ty, Some(Ty::Str)) {
        let mut to_sym = send(span, Some(arg.clone()), "to_sym", vec![]);
        to_sym.ty = Some(Ty::Sym);
        *arg = to_sym;
    }
}

/// The private helpers whose value is a nested `params.permit` (`def
/// moderation_params = @moderation_params ||= params.permit(:page, what:
/// [...])`), so their call sites can carry the String-keyed type.
pub(crate) fn filtered_permit_helpers(privs: &[crate::dialect::Action]) -> crate::det::HashSet<Symbol> {
    fn tail(e: &Expr) -> &Expr {
        match &*e.node {
            ExprNode::Seq { exprs } if !exprs.is_empty() => tail(exprs.last().unwrap()),
            ExprNode::OpAssign { value, .. } | ExprNode::Assign { value, .. } => tail(value),
            _ => e,
        }
    }
    privs
        .iter()
        .filter(|a| a.params.fields.is_empty() && filtered_chain(tail(&a.body)).is_some())
        .map(|a| a.name.clone())
        .collect()
}

/// `moderation_params.dig(:what, :stories)` → `.dig("what", "stories")`,
/// the indifferent read Rails' Parameters gives on String-keyed storage.
/// Not left to the Ruby emit's typed key coercion: these helper calls reach it untyped.
pub(crate) fn stringify_filtered_helper_reads(expr: &mut Expr, helpers: &crate::det::HashSet<Symbol>) {
    expr.node.for_each_child_mut(&mut |c| stringify_filtered_helper_reads(c, helpers));
    let ExprNode::Send { recv: Some(r), method, args, .. } = &mut *expr.node else { return };
    let helper_call = matches!(&*r.node, ExprNode::Send { recv, method, args, block: None, .. }
        if args.is_empty()
            && helpers.contains(method)
            && recv.as_ref().is_none_or(|r| matches!(&*r.node, ExprNode::SelfRef)));
    if !helper_call {
        return;
    }
    let keys = match method.as_str() {
        "dig" => args.len(),
        "[]" | "fetch" | "key?" | "has_key?" | "include?" | "member?" => args.len().min(1),
        _ => 0,
    };
    for key in args.iter_mut().take(keys) {
        if let Some(name) = sym_name(key) {
            *key = str_lit(key.span, name);
        }
    }
}

/// `params.permit(:a, what: [:x, :y])` → `Params.filtered_values(params,
/// ["a"], {"what" => ["x", "y"]}, {"what" => "any"})`.
/// Not the Symbol-keyed `permitted` hash: a nested filter's value is a
/// hash or list, which a `Hash[Symbol, String]` cannot hold, so this keeps
/// Rails' own String-keyed storage and the emit's indifferent key reads.
fn filtered_chain(e: &Expr) -> Option<Expr> {
    let ExprNode::Send { recv: Some(r), method, block: None, .. } = &*e.node else { return None };
    if method.as_str() != "permit" || !is_request_params(r) || is_param_value(r.ty.as_ref()) {
        return None;
    }
    filtered_call(e)
}

/// `Params.filtered_values` for a `permit` call whose last argument is a
/// Hash of nested filters.
fn filtered_call(e: &Expr) -> Option<Expr> {
    use crate::lower::controller_to_library::params::{nested_candidates, params_hash_ty};
    let ExprNode::Send { recv: Some(r), args, .. } = &*e.node else { return None };
    let (last, scalars) = args.split_last()?;
    let ExprNode::Hash { entries, .. } = &*last.node else { return None };
    let names = filter_names(scalars)?;
    let nested = nested_candidates(e, &crate::ident::ClassId(Symbol::from("")));
    // A key the parser could not read (`k: [{ … }]`) would be dropped silently.
    if entries.is_empty() || nested.len() != entries.len() {
        return None;
    }
    let span = e.span;
    let str_array = |items: Vec<String>| {
        let mut a = Expr::new(span, ExprNode::Array {
            elements: items.into_iter().map(|s| str_lit(span, s)).collect(),
            style: Default::default(),
        });
        a.ty = Some(Ty::Array { elem: std::sync::Arc::new(Ty::Str) });
        a
    };
    let hash = |entries: Vec<(Expr, Expr)>, value: Ty| {
        let mut h = Expr::new(span, ExprNode::Hash { entries, kwargs: false });
        h.ty = Some(Ty::Hash { key: std::sync::Arc::new(Ty::Str), value: std::sync::Arc::new(value) });
        h
    };
    let children = hash(
        nested.iter().map(|n| (str_lit(span, n.name.as_str().to_string()), str_array(n.allowed.clone()))).collect(),
        Ty::Array { elem: std::sync::Arc::new(Ty::Str) },
    );
    let modes = hash(
        nested.iter().map(|n| (str_lit(span, n.name.as_str().to_string()), str_lit(span, n.mode.to_string()))).collect(),
        Ty::Str,
    );
    let params_const = Expr::new(span, ExprNode::Const { path: vec![Symbol::from("Params")] });
    let mut out = send(span, Some(params_const), "filtered_values", vec![r.clone(), str_array(names), children, modes]);
    out.ty = Some(params_hash_ty());
    Some(out)
}

/// `params.permit(:a, :b)` on the request's params →
/// `Params.permitted(Params.permitted({}, params, "a", :a), params, "b", :b)`.
/// A model write reads Symbol keys, so this builds a Symbol-keyed hash
/// of the scalars the request provided, where `slice` would have handed
/// it String keys it never reads.
fn permitted_chain(e: &Expr) -> Option<Expr> {
    let ExprNode::Send { recv: Some(r), method, args, block: None, .. } = &*e.node else { return None };
    if method.as_str() != "permit" || args.is_empty() || !is_request_params(r) || is_param_value(r.ty.as_ref()) {
        return None;
    }
    let names = filter_names(args)?;
    let params_const = || Expr::new(e.span, ExprNode::Const { path: vec![Symbol::from("Params")] });
    let mut acc = Expr::new(e.span, ExprNode::Hash { entries: vec![], kwargs: false });
    acc.ty = Some(permitted_ty());
    for name in names {
        let sym = Expr::new(e.span, ExprNode::Lit { value: Literal::Sym { value: Symbol::from(name.as_str()) } });
        acc = send(e.span, Some(params_const()), "permitted", vec![acc, r.clone(), str_lit(e.span, name), sym]);
        acc.ty = Some(permitted_ty());
    }
    Some(acc)
}

/// What `Params.permitted` answers: the Symbol-keyed scalars.
fn permitted_ty() -> Ty {
    Ty::Hash { key: std::sync::Arc::new(Ty::Sym), value: std::sync::Arc::new(Ty::Str) }
}

/// The names a `permit` filter list spells, as Rails' `permit(*filters)` reads them: a Symbol or a String literal, or an
/// Array literal of those (`permit(%w[a b])`, `permit("a", "b")`), flattened. `None` for anything else.
fn filter_names(args: &[Expr]) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for arg in args {
        match &*arg.node {
            ExprNode::Array { elements, .. } => names.extend(filter_names(elements)?),
            _ => names.push(filter_name(arg)?),
        }
    }
    Some(names)
}

fn filter_name(e: &Expr) -> Option<String> {
    match &*e.node {
        ExprNode::Lit { value: Literal::Str { value } } => Some(value.clone()),
        _ => sym_name(e),
    }
}

/// `Params.allowed_scalars(recv, ["a", "b"])`: the named keys of `recv` whose values are not a Hash or an Array.
fn allowed_scalars(span: crate::span::Span, recv: &Expr, names: Vec<String>) -> Expr {
    let mut list = Expr::new(span, ExprNode::Array {
        elements: names.into_iter().map(|n| str_lit(span, n)).collect(),
        style: Default::default(),
    });
    list.ty = Some(Ty::Array { elem: std::sync::Arc::new(Ty::Str) });
    let params_const = Expr::new(span, ExprNode::Const { path: vec![Symbol::from("Params")] });
    send(span, Some(params_const), "allowed_scalars", vec![recv.clone(), list])
}

fn sym_name(e: &Expr) -> Option<String> {
    match &*e.node {
        ExprNode::Lit { value: Literal::Sym { value } } => Some(value.as_str().to_string()),
        _ => None,
    }
}

fn str_lit(span: crate::span::Span, value: String) -> Expr {
    let mut e = Expr::new(span, ExprNode::Lit { value: Literal::Str { value } });
    e.ty = Some(Ty::Str);
    e
}

fn send(span: crate::span::Span, recv: Option<Expr>, method: &str, args: Vec<Expr>) -> Expr {
    Expr::new(
        span,
        ExprNode::Send { recv, method: Symbol::from(method), args, block: None, parenthesized: true },
    )
}

/// Rails' `Parameters` API as a gem writes it, on the Hash the emitted program holds:
/// `Parameters.new(hash)` is the hash, a nested Parameters is a Hash (`respond_to?(:permit)`), a key a Symbol
/// names is its String (`@params[@resource_name]`), and `permit(*names)` keeps the scalars under those names.
fn parameters_on_hash(expr: &mut Expr) -> bool {
    let span = expr.span;
    let ExprNode::Send { recv: Some(r), method, args, block: None, .. } = &mut *expr.node else { return false };
    let is_parameters_const = matches!(&*r.node, ExprNode::Const { path }
        if path.iter().map(|s| s.as_str()).filter(|s| !s.is_empty()).collect::<Vec<_>>() == ["ActionController", "Parameters"]);
    // Not a fresh Request: Rails' wraps the env, whose state is the request the routes serve (Devise's warden_compat).
    let is_request_const = matches!(&*r.node, ExprNode::Const { path }
        if path.iter().map(|s| s.as_str()).filter(|s| !s.is_empty()).collect::<Vec<_>>() == ["ActionDispatch", "Request"]);
    if is_request_const && method.as_str() == "new" && args.len() == 1 {
        *method = Symbol::from("for_env");
        return true;
    }
    if is_parameters_const && method.as_str() == "new" {
        if let [hash] = args.as_slice() {
            let mut hash = hash.clone();
            hash.ty = expr.ty.clone();
            *expr = hash;
            return true;
        }
        return false;
    }
    if method.as_str() == "respond_to?" && holds_parameters(r.ty.as_ref()) && let [name] = args.as_slice() && sym_name(name).as_deref() == Some("permit") {
        let hash = Expr::new(span, ExprNode::Const { path: vec![Symbol::from("Hash")] });
        let mut test = send(span, Some(r.clone()), "is_a?", vec![hash]);
        test.ty = Some(Ty::Bool);
        *expr = test;
        return true;
    }
    // `permit(*names)` on a value that may be Parameters (Devise's `permit_keys(parameters, keys)`): only Parameters answers it.
    if method.as_str() == "permit"
        && holds_parameters(r.ty.as_ref())
        && matches!(&*r.node, ExprNode::Var { .. } | ExprNode::Ivar { .. })
        && let [arg] = args.as_slice()
        && let ExprNode::Splat { value } = &*arg.node
    {
        let names = value.clone();
        let params = Expr::new(span, ExprNode::Const { path: vec![Symbol::from("Params")] });
        let ty = expr.ty.clone();
        *expr = send(span, Some(params), "permitted_names", vec![r.clone(), names]);
        expr.ty = ty;
        return true;
    }
    // Not a String read of a value that may be either: Parameters hold String keys here, the Hash beside them Symbols.
    let value_holder = !matches!(&*r.node, ExprNode::Send { method, .. } if method.as_str() == "params");
    // A Symbol read of a request value that may be Parameters among scalars (Devise's `attributes[:reset_password_token]`
    // over `params.fetch(resource_name, {})`): only Parameters answer it, a String or an Array raising as in Rails.
    let symbol_read = matches!(method.as_str(), "[]" | "delete") && matches!(args.as_slice(), [k] if matches!(&*k.node, ExprNode::Lit { value: Literal::Sym { .. } }));
    if value_holder && (may_be_parameters(r.ty.as_ref()) || (symbol_read && holds_parameters(r.ty.as_ref()))) && !is_parameters(r.ty.as_ref()) {
        // Not a literal String key, which reads alike either way.
        let symbol_key = |k: &Expr| !matches!(&*k.node, ExprNode::Lit { value: Literal::Str { .. } }) && !matches!(k.ty, Some(Ty::Str | Ty::Int));
        if matches!(method.as_str(), "[]" | "delete") && let [key] = args.as_slice() && symbol_key(key) {
            let params = Expr::new(span, ExprNode::Const { path: vec![Symbol::from("Params")] });
            let ty = expr.ty.clone();
            let via = if method.as_str() == "delete" { "indifferent_delete" } else { "indifferent_read" };
            *expr = send(span, Some(params), via, vec![r.clone(), key.clone()]);
            expr.ty = ty;
            return true;
        }
        return false;
    }
    if !is_parameters(r.ty.as_ref()) && !is_request_params_read(r) {
        return false;
    }
    match (method.as_str(), args.as_mut_slice()) {
        ("respond_to?", [name]) if sym_name(name).as_deref() == Some("permit") => {
            let hash = Expr::new(span, ExprNode::Const { path: vec![Symbol::from("Hash")] });
            let mut test = send(span, Some(r.clone()), "is_a?", vec![hash]);
            test.ty = Some(Ty::Bool);
            *expr = test;
            true
        }
        // A Symbol literal read or removed on Parameters a method is handed (Devise's `update_with_password(params)`,
        // a plain class's `initialize(params)`): either key, as Rails' Parameters answer, since the request's Hash holds
        // the String and a `permit` the emit lowered the Symbol. Not a controller's own `params`, which the emit's
        // coercion writes as Rails reads it.
        ("[]" | "delete", [key]) if matches!(&*r.node, ExprNode::Var { .. }) && matches!(&*key.node, ExprNode::Lit { value: Literal::Sym { .. } }) => {
            let params = Expr::new(span, ExprNode::Const { path: vec![Symbol::from("Params")] });
            let ty = expr.ty.clone();
            let via = if method.as_str() == "delete" { "indifferent_delete" } else { "indifferent_read" };
            *expr = send(span, Some(params), via, vec![r.clone(), key.clone()]);
            expr.ty = ty;
            true
        }
        // Any other receiver of Parameters (Warden's `params_auth_hash[:remember_me]`) holds the request's String keys.
        ("[]" | "delete" | "fetch" | "key?" | "has_key?", [key, ..])
            if let ExprNode::Lit { value: Literal::Sym { value } } = &*key.node =>
        {
            let mut text = Expr::new(key.span, ExprNode::Lit { value: Literal::Str { value: value.as_str().to_string() } });
            text.ty = Some(Ty::Str);
            *key = text;
            false
        }
        // Not a literal key: the emit's coercion writes those as Rails reads them.
        ("[]" | "fetch" | "key?" | "has_key?", [key, ..])
            if !matches!(key.ty, Some(Ty::Str)) && !matches!(&*key.node, ExprNode::Lit { .. }) =>
        {
            let sym = key.clone();
            let mut text = send(span, Some(sym), "to_s", vec![]);
            text.ty = Some(Ty::Str);
            *key = text;
            false
        }
        // Not a request's `params.require(:x).permit(*names)`: the controller lowering types that one.
        ("permit", [arg]) if let ExprNode::Splat { value } = &*arg.node && matches!(&*r.node, ExprNode::Var { .. } | ExprNode::Ivar { .. }) => {
            let names = value.clone();
            let params = Expr::new(span, ExprNode::Const { path: vec![Symbol::from("Params")] });
            let ty = expr.ty.clone();
            *expr = send(span, Some(params), "permitted_names", vec![r.clone(), names]);
            expr.ty = ty;
            true
        }
        _ => false,
    }
}
