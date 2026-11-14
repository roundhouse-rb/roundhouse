//! Native Sidekiq workers under `queue_adapter :sidekiq`: a class that
//! includes `Sidekiq::Job` or `Sidekiq::Worker` pushes its jobs to Redis
//! in Sidekiq's own envelope, the way Sidekiq 8.1's client writes them,
//! instead of running them inline as it does for every other app.
//!
//! Four halves, run where `job_payload`'s are:
//!
//! - [`fold_sites`] rewrites each enqueue of a worker into one call,
//!   `W.perform_async_with(at, queue, retry, shape, argc, args…)`:
//!   `perform_async`, `perform_in` / `perform_at`, and any chain of
//!   literal `set(queue:, wait:, wait_until:, retry:)` before one of
//!   them. `at` is the interval or epoch the call names (0.0 for none),
//!   still to go through Sidekiq's rule for which it is; `queue` and
//!   `retry` are a `set` override or "" for the class's own; `shape` is
//!   how Sidekiq builds the envelope: 0 through its `Setter` (`args`
//!   before `class`), 1 for a bare class-side `perform_in` /
//!   `perform_at` (`class` before `args`), 2 for one job of a bulk push,
//!   held for its batch; `argc` is how many arguments the call passed,
//!   since Sidekiq writes only those.
//!   Run from `job_class_side`, on every target.
//! - [`plan_workers`] works out a [`NativePlan`] per worker from those
//!   call sites: the JSON codec for each of `perform`'s parameters, and
//!   the envelope's head from the class's literal `sidekiq_options`
//!   (its ancestors' first). Recorded on `App::native_plans`.
//! - [`apply_ruby`] rewrites the enqueue arm of each planned worker's
//!   `perform_async` and `perform_async_with` from the Proc to
//!   `JobRegistry.push_native_<worker>(…)`, and files the planning
//!   errors. Ruby emitter only, as `job_payload::apply_ruby`.
//! - [`registry_source`] writes each `push_native_<worker>` into
//!   `JobRegistry`, beside the ActiveJob writers.
//!
//! The bytes are held to a recording of real Sidekiq 8.1.7
//! (`tests/sidekiq_native/sidekiq.json`) by
//! `tests/emit_and_run/sidekiq_native_push.rs`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::dialect::{LibraryClass, MethodDef, MethodReceiver};
use crate::diagnostic::{Diagnostic, Severity};
use crate::expr::{Expr, ExprNode, Literal};
use crate::ident::Symbol;
use crate::span::Span;
use crate::ty::Ty;

/// How one of `perform`'s arguments is written as native JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NativeCodec {
    Bool,
    Int,
    Float,
    Str,
    /// Only ever `nil` at the call sites.
    Null,
    /// A value of the inner codec, or `nil`.
    Nullable(Box<NativeCodec>),
    Array(Box<NativeCodec>),
    /// A Hash with String keys.
    Hash(Box<NativeCodec>),
    /// Any JSON scalar, its kind known only when it is written: a
    /// String, Integer, Float, true, false or nil. Call sites that pass
    /// different scalars here, or a Hash whose values are different
    /// scalars, as Sidekiq accepts.
    Scalar,
    /// No call site passes this (optional) parameter, so it is never
    /// written: the argument list stops before it. Inside an Array or a
    /// Hash while planning, an element type still unknown (an empty
    /// literal), which a join with another site fills.
    Omitted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NativeParam {
    pub name: Symbol,
    pub codec: NativeCodec,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NativePlan {
    /// `ActivityPub::DeliveryWorker`.
    pub worker: String,
    /// The envelope's `retry`, as JSON text (`true` by default).
    pub retry: String,
    /// The class's queue (`default` by default).
    pub queue: String,
    /// The class's other literal `sidekiq_options`, in the order Sidekiq
    /// writes them, as `,"key":value` JSON text.
    pub extra: String,
    pub params: Vec<NativeParam>,
    /// Set when the worker can't be pushed: why. `params` is then empty.
    pub fallback: Option<String>,
    /// The call site the fallback is about, when one is: where its error is filed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_site: Option<Span>,
}

impl NativePlan {
    pub fn is_payload(&self) -> bool {
        self.fallback.is_none()
    }

    pub fn suffix(&self) -> String {
        crate::naming::underscore(&self.worker).replace('/', "__")
    }
}

/// The `sidekiq_options` keys the worker or the envelope's readers act
/// on. Any other literal key is still carried, as Sidekiq carries it,
/// with a warning that nothing here honors it.
const HONORED_KEYS: &[&str] = &["queue", "retry", "dead", "backtrace", "retry_queue", "retry_for"];

/// A worker's literal `sidekiq_options`, its ancestors' first and a later
/// key replacing an earlier one in place, each value as JSON text. Plus
/// the diagnostics:
///
/// - an error for a `sidekiq_options` that isn't one literal Hash, and
///   for an honored key (`queue:`, `retry:`, …) whose value isn't a
///   literal, the worker's own or inherited: the push would write the
///   default in its place, so the job would go to another queue or retry
///   another way;
/// - a warning for any other key whose value isn't a literal (not
///   carried), the worker's own or inherited, and for a key of its own
///   that is carried but not honored.
pub fn options(app: &App, lc: &LibraryClass) -> (Vec<(String, String)>, Vec<Diagnostic>) {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut diags = Vec::new();
    let worker = lc.name.0.as_str();
    for class in ancestry(app, lc).iter().rev() {
        let own = class.name == lc.name;
        let whose = if own { String::new() } else { format!(" (inherited from `{}`)", class.name.0.as_str()) };
        for call in &class.unknown_calls {
            let ExprNode::Send { recv: None, method, args, block: None, .. } = &*call.node else { continue };
            if method.as_str() != "sidekiq_options" {
                continue;
            }
            let entries = match args.as_slice() {
                [] => continue,
                [opts] => match &*opts.node {
                    ExprNode::Hash { entries, .. } => Some(entries),
                    _ => None,
                },
                _ => None,
            };
            let Some(entries) = entries else {
                diags.push(error(
                    call.span,
                    "worker-options",
                    "options not a literal Hash",
                    format!(
                        "sidekiq_native: `{worker}`'s sidekiq_options{whose} is not one literal Hash, so its queue and retry can't be known; under queue_adapter :sidekiq the push writes them into the envelope"
                    ),
                ));
                continue;
            };
            for (k, v) in entries {
                let key = match &*k.node {
                    ExprNode::Lit { value: Literal::Sym { value } } => value.as_str().to_string(),
                    ExprNode::Lit { value: Literal::Str { value } } => value.clone(),
                    _ => continue,
                };
                let Some(json) = literal_json(v) else {
                    if HONORED_KEYS.contains(&key.as_str()) {
                        diags.push(error(
                            k.span,
                            "worker-options",
                            "not a literal option",
                            format!(
                                "sidekiq_native: `{worker}`'s sidekiq_options `{key}:`{whose} is not a literal; under queue_adapter :sidekiq the push writes it into the envelope, and would write the default in its place"
                            ),
                        ));
                    } else {
                        diags.push(warning(
                            k.span,
                            "worker-options",
                            "not a literal option",
                            format!(
                                "sidekiq_native: `{worker}`'s sidekiq_options `{key}:`{whose} is not a literal, so it is not carried into the envelope"
                            ),
                        ));
                    }
                    continue;
                };
                if own && !HONORED_KEYS.contains(&key.as_str()) {
                    diags.push(warning(
                        k.span,
                        "worker-options",
                        "option carried, not honored",
                        format!(
                            "sidekiq_native: `{}`'s sidekiq_options `{key}:` is carried into the envelope; no middleware honors it here",
                            lc.name.0.as_str()
                        ),
                    ));
                }
                match out.iter_mut().find(|(have, _)| *have == key) {
                    Some(slot) => slot.1 = json,
                    None => out.push((key, json)),
                }
            }
        }
    }
    (out, diags)
}

/// A literal as the JSON Sidekiq writes for it: Symbols as Strings.
fn literal_json(e: &Expr) -> Option<String> {
    match &*e.node {
        ExprNode::Lit { value } => match value {
            Literal::Nil => Some("null".to_string()),
            Literal::Bool { value } => Some(value.to_string()),
            Literal::Int { value } => Some(value.to_string()),
            Literal::Float { value } if value.is_finite() => Some(ruby_float(*value)),
            Literal::Str { value } => Some(json_str(value)),
            Literal::Sym { value } => Some(json_str(value.as_str())),
            _ => None,
        },
        ExprNode::Array { elements, .. } => {
            let parts: Option<Vec<String>> = elements.iter().map(literal_json).collect();
            Some(format!("[{}]", parts?.join(",")))
        }
        _ => None,
    }
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

fn ruby_float(v: f64) -> String {
    let s = v.to_string();
    if s.contains('.') || s.contains('e') { s } else { format!("{s}.0") }
}

/// The class, then its parents within the ingested set.
fn ancestry<'a>(app: &'a App, lc: &'a LibraryClass) -> Vec<&'a LibraryClass> {
    let mut out = vec![lc];
    let mut cursor = lc.parent.clone();
    while let Some(p) = cursor {
        if out.iter().any(|c| c.name == p) {
            break;
        }
        let Some(parent) = app.library_classes.iter().find(|c| c.name == p) else { break };
        out.push(parent);
        cursor = parent.parent.clone();
    }
    out
}

/// Where a worker's own `perform_async` comes from: the method, and the
/// module it extends that provides it, if it isn't defined on the class
/// or an ancestor.
pub struct OwnAsync {
    pub span: Span,
    pub module: Option<String>,
}

/// The workers in `workers` that have a class-side `perform_async` of
/// their own: defined on the class or an ancestor, or an instance method
/// of an app module one of them `extend`s. Their calls aren't folded, so
/// they keep dispatching to it; enqueueing one under `:sidekiq` is an
/// error ([`own_perform_async_error`]). A module the app doesn't define
/// (a gem's) isn't looked into.
pub fn own_perform_async(app: &App, workers: &BTreeSet<String>) -> std::collections::BTreeMap<String, OwnAsync> {
    let defined = |c: &LibraryClass, receiver: MethodReceiver| {
        c.methods.iter().find(|m| m.receiver == receiver && m.name.as_str() == "perform_async").map(|m| m.name_span)
    };
    // As Ruby resolves a constant in a class body: nested in the class,
    // then in each enclosing namespace, innermost first, then at the top
    // level. A leading `::` names the top level only.
    let module_named = |owner: &str, path: &[Symbol]| -> Option<&LibraryClass> {
        let written = path.iter().map(|s| s.as_str()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("::");
        let rooted = path.first().is_some_and(|s| s.as_str().is_empty());
        let mut candidates: Vec<String> = Vec::new();
        if !rooted {
            let mut scope: Vec<&str> = owner.split("::").collect();
            while !scope.is_empty() {
                candidates.push(format!("{}::{written}", scope.join("::")));
                scope.pop();
            }
        }
        candidates.push(written);
        candidates
            .iter()
            .find_map(|name| app.library_classes.iter().find(|c| c.is_module && c.name.0.as_str() == name.as_str()))
    };
    app.library_classes
        .iter()
        .filter(|lc| workers.contains(lc.name.0.as_str()))
        .filter_map(|lc| {
            let found = ancestry(app, lc).into_iter().find_map(|c| {
                if let Some(span) = defined(c, MethodReceiver::Class) {
                    return Some(OwnAsync { span, module: None });
                }
                c.unknown_calls.iter().rev().find_map(|call| {
                    let ExprNode::Send { recv: None, method, args, .. } = &*call.node else { return None };
                    if method.as_str() != "extend" {
                        return None;
                    }
                    args.iter().rev().find_map(|a| {
                        let ExprNode::Const { path } = &*a.node else { return None };
                        let module = module_named(c.name.0.as_str(), path)?;
                        let span = defined(module, MethodReceiver::Instance)?;
                        Some(OwnAsync { span, module: Some(module.name.0.as_str().to_string()) })
                    })
                })
            })?;
            Some((lc.name.0.as_str().to_string(), found))
        })
        .collect()
}

/// A worker with its own `perform_async`, enqueued under `:sidekiq`: its
/// jobs would run where that method sends them, not in Redis.
pub fn own_perform_async_error(worker: &str, own: &OwnAsync) -> Diagnostic {
    let has = match &own.module {
        None => "defines its own `perform_async`".to_string(),
        Some(module) => format!("gets its own `perform_async` from `{module}`"),
    };
    error(
        own.span,
        "job-own-perform-async",
        "own perform_async not lowered",
        format!(
            "sidekiq_native: `{worker}` {has}, which isn't lowered to a push; under queue_adapter :sidekiq its jobs must be pushed to Redis in Sidekiq's envelope"
        ),
    )
}

/// The `perform` a worker runs: its own, or the nearest ancestor's.
pub fn inherited_perform(app: &App, lc: &LibraryClass) -> Option<MethodDef> {
    ancestry(app, lc).into_iter().find_map(|c| {
        c.methods
            .iter()
            .find(|m| m.receiver == MethodReceiver::Instance && m.name.as_str() == "perform")
            .cloned()
    })
}

fn warning(span: Span, key: &str, reason: &str, message: String) -> Diagnostic {
    crate::lower::residue_diagnostic("sidekiq_native", key, span, reason, message)
}

fn error(span: Span, key: &str, reason: &str, message: String) -> Diagnostic {
    let mut d = warning(span, key, reason, message);
    d.severity = Severity::Error;
    d
}

// ---- Folding the call sites ------------------------------------------

/// The number of lead arguments `perform_async_with` takes before the
/// job's own.
pub const LEAD: usize = 5;

/// `perform_async_with`'s `shape`: through Sidekiq's `Setter` (`args`
/// before `class`).
const SHAPE_SETTER: i64 = 0;
/// A bare class-side `perform_in` / `perform_at` (`class` before `args`).
const SHAPE_CLASS_SIDE: i64 = 1;
/// One job of a bulk push: held for its batch, and only such a job is.
const SHAPE_BULK_ITEM: i64 = 2;

/// What [`fold_sites`] did: the workers it folded a call for, and each
/// `set(...)` it couldn't honor.
#[derive(Default)]
pub struct NativeFolds {
    pub workers: BTreeSet<String>,
    /// The workers a bulk push was folded for, which get
    /// `push_bulk_begin` and `push_bulk_end`.
    pub bulk: BTreeSet<String>,
    pub rejected: Vec<Span>,
    pub rejected_bulk: Vec<Span>,
}

fn typed(span: Span, node: ExprNode, ty: Ty) -> Expr {
    let mut e = Expr::new(span, node);
    e.ty = Some(ty);
    e
}

fn float_lit(span: Span, v: f64) -> Expr {
    typed(span, ExprNode::Lit { value: Literal::Float { value: v } }, Ty::Float)
}

fn str_lit(span: Span, v: &str) -> Expr {
    typed(span, ExprNode::Lit { value: Literal::Str { value: v.to_string() } }, Ty::Str)
}

fn int_lit(span: Span, v: i64) -> Expr {
    typed(span, ExprNode::Lit { value: Literal::Int { value: v } }, Ty::Int)
}

fn send(span: Span, recv: Expr, method: &str, args: Vec<Expr>, ty: Ty) -> Expr {
    typed(
        span,
        ExprNode::Send { recv: Some(recv), method: Symbol::from(method), args, block: None, parenthesized: false },
        ty,
    )
}

/// An interval or epoch as a Float: a literal number or Duration folded
/// here (so a target with no Duration still gets a plain Float), anything
/// else through `to_f`, which an Integer, a Float, a Duration and a Time
/// all answer.
fn seconds(v: &Expr) -> Expr {
    let span = v.span;
    match &*v.node {
        ExprNode::Lit { value: Literal::Int { value } } => float_lit(span, *value as f64),
        ExprNode::Lit { value: Literal::Float { value } } => float_lit(span, *value),
        _ => match super::duration::literal_seconds(v) {
            Some(n) => float_lit(span, n as f64),
            None => send(span, v.clone(), "to_f", vec![], Ty::Float),
        },
    }
}

/// The options a chain of `set(...)` calls leaves, later ones winning as
/// Sidekiq's `Setter` merges them.
struct SetOptions {
    at: Option<Expr>,
    queue: Option<Expr>,
    retry: Option<String>,
}

/// `None` when a `set` isn't literal Symbol keys among `queue:`,
/// `wait:`, `wait_until:` and `retry:` (a literal Boolean or Integer).
fn set_options(sets: &[&[Expr]]) -> Option<SetOptions> {
    let mut out = SetOptions { at: None, queue: None, retry: None };
    for args in sets {
        let [opts] = args else { return None };
        let ExprNode::Hash { entries, .. } = &*opts.node else { return None };
        let mut wait = None;
        let mut wait_until = None;
        for (k, v) in entries {
            let ExprNode::Lit { value: Literal::Sym { value: key } } = &*k.node else { return None };
            match key.as_str() {
                "wait" => wait = Some(seconds(v)),
                "wait_until" => wait_until = Some(seconds(v)),
                "queue" => {
                    out.queue = Some(match &*v.node {
                        ExprNode::Lit { value: Literal::Str { value } } => str_lit(v.span, value),
                        ExprNode::Lit { value: Literal::Sym { value } } => str_lit(v.span, value.as_str()),
                        _ => send(v.span, v.clone(), "to_s", vec![], Ty::Str),
                    })
                }
                "retry" => match &*v.node {
                    ExprNode::Lit { value: Literal::Bool { value } } => out.retry = Some(value.to_string()),
                    ExprNode::Lit { value: Literal::Int { value } } => out.retry = Some(value.to_string()),
                    _ => return None,
                },
                _ => return None,
            }
        }
        // `wait_until:` wins over `wait:` in one `set`: the Setter reads
        // `wait_until` first.
        if let Some(at) = wait_until.or(wait) {
            out.at = Some(at);
        }
    }
    Some(out)
}

/// Fold every enqueue of a worker in `async_workers` (and, for a bare
/// `perform_in` / `perform_at`, in `delay_workers`: those that don't
/// define their own) into `W.perform_async_with(...)`. A call that
/// splats its arguments is left alone, since its count isn't known.
pub fn fold_sites(
    e: &mut Expr,
    async_workers: &BTreeSet<String>,
    delay_workers: &BTreeSet<String>,
    folds: &mut NativeFolds,
    models: &NativeModels,
) {
    let mut env = Vec::new();
    fold_in(e, async_workers, delay_workers, folds, models, &mut env);
}

/// Each model's column types, for typing a bulk push's item.
#[derive(Default)]
pub struct NativeModels {
    columns: BTreeMap<String, BTreeMap<String, Ty>>,
}

pub fn native_models(app: &App) -> NativeModels {
    let mut columns = BTreeMap::new();
    for m in &app.models {
        let mut cols: BTreeMap<String, Ty> =
            m.attributes.fields.iter().map(|(k, v)| (k.as_str().to_string(), v.clone())).collect();
        cols.entry("id".to_string()).or_insert(Ty::Int);
        columns.insert(m.name.0.as_str().to_string(), cols);
    }
    NativeModels { columns }
}

/// The methods that keep a query's or a list's model: the chain from
/// `Status.where(…)` or `account.follow_requests` through to a batch.
const MODEL_CHAIN: &[&str] = &[
    "where", "not", "or", "order", "reorder", "preload", "includes", "eager_load", "joins", "left_joins", "limit",
    "offset", "all", "unscoped", "select", "filter", "reject", "to_a", "distinct", "merge", "sort_by", "uniq", "compact",
];

/// The model whose records `list` holds: from its type (an Array of a
/// model's records, or a relation), a local bound to one, a
/// `find_in_batches` batch, or a query chain on a model class.
fn list_model(list: &Expr, models: &NativeModels, env: &[(Symbol, String)]) -> Option<String> {
    let known = |id: &str| models.columns.contains_key(id).then(|| id.to_string());
    match &list.ty {
        Some(Ty::Array { elem }) => {
            if let Ty::Class { id, .. } = &**elem
                && let Some(m) = known(id.0.as_str())
            {
                return Some(m);
            }
        }
        Some(Ty::Relation { of }) => {
            if let Some(m) = known(of.0.as_str()) {
                return Some(m);
            }
        }
        _ => {}
    }
    match &*list.node {
        ExprNode::Var { name, .. } => env.iter().rev().find(|(n, _)| n == name).map(|(_, m)| m.clone()),
        ExprNode::Const { path } => {
            let name = path.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("::");
            known(name.trim_start_matches("::"))
        }
        ExprNode::Send { recv: Some(r), method, .. } if MODEL_CHAIN.contains(&method.as_str()) => list_model(r, models, env),
        _ => None,
    }
}

fn fold_in(
    e: &mut Expr,
    async_workers: &BTreeSet<String>,
    delay_workers: &BTreeSet<String>,
    folds: &mut NativeFolds,
    models: &NativeModels,
    env: &mut Vec<(Symbol, String)>,
) {
    let mark = env.len();
    // `<list>.find_in_batches do |batch|`: the batch holds the list's records.
    if let ExprNode::Send { recv: Some(r), method, block: Some(b), .. } = &*e.node
        && matches!(method.as_str(), "find_in_batches" | "in_batches")
        && let ExprNode::Lambda { params, .. } = &*b.node
        && let Some(p) = params.first()
        && let Some(m) = list_model(r, models, env)
    {
        env.push((p.clone(), m));
    }
    let worker_of = |r: &Expr, set: &BTreeSet<String>| -> Option<String> {
        let ExprNode::Const { path } = &*r.node else { return None };
        let name = path.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("::");
        let name = name.trim_start_matches("::").to_string();
        set.contains(&name).then_some(name)
    };
    let mut replacement: Option<Expr> = fold_bulk(e, async_workers, folds, models, env);
    if replacement.is_none()
        && let ExprNode::Send { recv: Some(recv), method, args, block: None, .. } = &*e.node
        && matches!(method.as_str(), "perform_async" | "perform_in" | "perform_at")
        && !args.iter().any(|a| matches!(&*a.node, ExprNode::Splat { .. } | ExprNode::KeywordSplat { .. }))
    {
        let delayed = method.as_str() != "perform_async";
        // Unwind `W.set(a).set(b)` to `W` and its sets, innermost first.
        let mut sets: Vec<&[Expr]> = Vec::new();
        let mut base = recv;
        while let ExprNode::Send { recv: Some(inner), method: m, args: set_args, block: None, .. } = &*base.node
            && m.as_str() == "set"
        {
            sets.push(set_args.as_slice());
            base = inner;
        }
        sets.reverse();
        // A `set` chain is the Setter's whichever entry point ends it,
        // so a worker's own class-side `perform_in` doesn't apply.
        let eligible = if delayed && sets.is_empty() { delay_workers } else { async_workers };
        if let Some(worker) = worker_of(base, eligible)
            && (!delayed || !args.is_empty())
        {
            let span = e.span;
            let parsed = if sets.is_empty() {
                Some(SetOptions { at: None, queue: None, retry: None })
            } else {
                set_options(&sets)
            };
            match parsed {
                None => folds.rejected.push(recv.span),
                Some(opts) => {
                    let mut own: Vec<Expr> = args.clone();
                    let at = if delayed { Some(seconds(&own.remove(0))) } else { opts.at };
                    let at = at.unwrap_or_else(|| float_lit(span, 0.0));
                    let queue = opts.queue.unwrap_or_else(|| str_lit(span, ""));
                    let retry = str_lit(span, opts.retry.as_deref().unwrap_or(""));
                    let shape = int_lit(span, if delayed && sets.is_empty() { SHAPE_CLASS_SIDE } else { SHAPE_SETTER });
                    let argc = int_lit(span, own.len() as i64);
                    let mut all = vec![at, queue, retry, shape, argc];
                    all.extend(own);
                    let mut call = Expr::new(
                        span,
                        ExprNode::Send {
                            recv: Some(base.clone()),
                            method: Symbol::from("perform_async_with"),
                            args: all,
                            block: None,
                            parenthesized: true,
                        },
                    );
                    call.ty = Some(Ty::Nil);
                    folds.workers.insert(worker);
                    replacement = Some(call);
                }
            }
        }
    }
    if let Some(r) = replacement {
        *e = r;
    }
    e.node.for_each_child_mut(&mut |c| fold_in(c, async_workers, delay_workers, folds, models, env));
    env.truncate(mark);
    // `kept = Status.where(…).reject { … }`: later reads of `kept` hold
    // the same model's records.
    if let ExprNode::Assign { target: crate::expr::LValue::Var { name, .. }, value } = &*e.node
        && let Some(m) = list_model(value, models, env)
    {
        env.push((name.clone(), m));
    }
}

/// `perform_async_with(0.0, <queue>, <retry>, 2, argc, row…)` on
/// `worker`: one job of a bulk push.
fn async_with_call(span: Span, worker: &Expr, queue: Expr, retry: &str, row: Vec<Expr>) -> Expr {
    let mut all = vec![float_lit(span, 0.0), queue, str_lit(span, retry), int_lit(span, SHAPE_BULK_ITEM), int_lit(span, row.len() as i64)];
    all.extend(row);
    typed(
        span,
        ExprNode::Send {
            recv: Some(worker.clone()),
            method: Symbol::from("perform_async_with"),
            args: all,
            block: None,
            parenthesized: true,
        },
        Ty::Nil,
    )
}

/// The trailing `name: <Integer literal>` of a bulk call, if it has one.
/// `Err` when the call ends in some other Hash.
fn bulk_limit(args: &mut Vec<Expr>, name: &str) -> Result<Option<i64>, ()> {
    let Some(last) = args.last() else { return Ok(None) };
    let ExprNode::Hash { entries, .. } = &*last.node else { return Ok(None) };
    let [(k, v)] = entries.as_slice() else { return Err(()) };
    let (ExprNode::Lit { value: Literal::Sym { value: key } }, ExprNode::Lit { value: Literal::Int { value: n } }) =
        (&*k.node, &*v.node)
    else {
        return Err(());
    };
    if key.as_str() != name || *n <= 0 {
        return Err(());
    }
    let n = *n;
    args.pop();
    Ok(Some(n))
}

/// A bulk push, folded to one `perform_async_with` per job (shape
/// `SHAPE_BULK_ITEM`) between the worker's `push_bulk_begin(limit)` and
/// `push_bulk_end`, with `push_bulk_abort` on every exit, which batch
/// them as Sidekiq's `Client#push_bulk` does (`SidekiqPush.bulk_begin`):
///
/// - `W.push_bulk(items, limit: N) { |x| …; [a, b] }` (the sidekiq-bulk
///   gem): the block's last expression, an Array literal, is each job's
///   arguments; with no block each item is the one argument. `N` is
///   10,000 when not given, as the gem's.
/// - `W.perform_bulk([[a], [b]], batch_size: N)`, after any literal
///   `set(queue:, retry:)`: a literal list of literal argument lists,
///   1,000 to a batch when not given, as Sidekiq's.
///
/// On a target without the push the jobs run one by one, inline.
/// Anything else is left as written and filed as an error
/// (`rejected_bulk`): Sidekiq would push those jobs.
fn fold_bulk(
    e: &Expr,
    workers: &BTreeSet<String>,
    folds: &mut NativeFolds,
    models: &NativeModels,
    env: &[(Symbol, String)],
) -> Option<Expr> {
    let ExprNode::Send { recv: Some(recv), method, args, block, .. } = &*e.node else { return None };
    let push_bulk = match method.as_str() {
        "push_bulk" => true,
        "perform_bulk" => false,
        _ => return None,
    };
    let mut sets: Vec<&[Expr]> = Vec::new();
    let mut base = recv;
    while let ExprNode::Send { recv: Some(inner), method: m, args: set_args, block: None, .. } = &*base.node
        && m.as_str() == "set"
    {
        sets.push(set_args.as_slice());
        base = inner;
    }
    sets.reverse();
    let ExprNode::Const { path } = &*base.node else { return None };
    let name = path.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("::");
    let name = name.trim_start_matches("::").to_string();
    if !workers.contains(&name) {
        return None;
    }
    let span = e.span;
    let reject = |folds: &mut NativeFolds| {
        folds.rejected_bulk.push(span);
        None
    };
    let opts = if sets.is_empty() { Some(SetOptions { at: None, queue: None, retry: None }) } else { set_options(&sets) };
    let Some(opts) = opts.filter(|o| o.at.is_none() && !(push_bulk && !sets.is_empty())) else { return reject(folds) };
    let queue = || opts.queue.clone().unwrap_or_else(|| str_lit(span, ""));
    let retry = opts.retry.clone().unwrap_or_default();
    let mut args = args.clone();
    let Ok(limit) = bulk_limit(&mut args, if push_bulk { "limit" } else { "batch_size" }) else { return reject(folds) };
    let [list] = args.as_slice() else { return reject(folds) };
    let body = if push_bulk {
        // `items.each { |x| …; W.perform_async_with(…, a, b) }`.
        let each_block = match block {
            None => {
                let mut item = Expr::new(span, ExprNode::Var { id: crate::ident::VarId(0), name: Symbol::from("item") });
                item.ty = match &list.ty {
                    Some(Ty::Array { elem }) => Some((**elem).clone()),
                    _ => None,
                };
                Expr::new(
                    span,
                    ExprNode::Lambda {
                        extra_params: Vec::new(),
                        params: vec![Symbol::from("item")],
                        rest_param: None,
                        block_param: None,
                        body: async_with_call(span, base, queue(), &retry, vec![item]),
                        block_style: crate::expr::BlockStyle::Brace,
                    },
                )
            }
            Some(b) => {
                let ExprNode::Lambda { params, rest_param: None, block_param: None, extra_params, body, block_style } = &*b.node
                else {
                    return reject(folds);
                };
                if params.len() != 1 || !extra_params.is_empty() {
                    return reject(folds);
                }
                // `push_bulk` is no method the analyzer knows, so the
                // block's parameter is untyped: it is each item of `list`.
                let mut body = body.clone();
                // When `list` holds a model's records, each item is one,
                // and its attribute reads take the model's column types.
                match list_model(list, models, env) {
                    Some(model) => retype_item(&mut body, &params[0], &model, models),
                    None => {
                        if let Some(Ty::Array { elem }) = &list.ty {
                            retype_var(&mut body, &params[0], elem);
                        }
                    }
                }
                let mut stmts: Vec<Expr> = match &*body.node {
                    ExprNode::Seq { exprs } => exprs.clone(),
                    _ => vec![body.clone()],
                };
                let Some(last) = stmts.pop() else { return reject(folds) };
                let ExprNode::Array { elements, .. } = &*last.node else { return reject(folds) };
                if elements.iter().any(|a| matches!(&*a.node, ExprNode::Splat { .. })) {
                    return reject(folds);
                }
                stmts.push(async_with_call(last.span, base, queue(), &retry, elements.clone()));
                let new_body = if stmts.len() == 1 {
                    stmts.pop().unwrap()
                } else {
                    typed(body.span, ExprNode::Seq { exprs: stmts }, Ty::Nil)
                };
                Expr::new(
                    b.span,
                    ExprNode::Lambda {
                        extra_params: Vec::new(),
                        params: params.clone(),
                        rest_param: None,
                        block_param: None,
                        body: new_body,
                        block_style: block_style.clone(),
                    },
                )
            }
        };
        let mut each = Expr::new(
            span,
            ExprNode::Send {
                recv: Some(list.clone()),
                method: Symbol::from("each"),
                args: vec![],
                block: Some(each_block),
                parenthesized: false,
            },
        );
        each.ty = list.ty.clone();
        each
    } else {
        // `perform_bulk([[1], [2]])`: one call per literal row.
        if block.is_some() {
            return reject(folds);
        }
        let ExprNode::Array { elements: rows, .. } = &*list.node else { return reject(folds) };
        let mut calls = Vec::new();
        for row in rows {
            let ExprNode::Array { elements, .. } = &*row.node else { return reject(folds) };
            if elements.iter().any(|a| matches!(&*a.node, ExprNode::Splat { .. })) {
                return reject(folds);
            }
            calls.push(async_with_call(row.span, base, queue(), &retry, elements.clone()));
        }
        typed(span, ExprNode::Seq { exprs: calls }, Ty::Nil)
    };
    let limit = limit.unwrap_or(if push_bulk { 10_000 } else { 1_000 });
    let begin = typed(
        span,
        ExprNode::Send {
            recv: Some(base.clone()),
            method: Symbol::from("push_bulk_begin"),
            args: vec![int_lit(span, limit)],
            block: None,
            parenthesized: true,
        },
        Ty::Nil,
    );
    let end = typed(
        span,
        ExprNode::Send { recv: Some(base.clone()), method: Symbol::from("push_bulk_end"), args: vec![], block: None, parenthesized: false },
        Ty::Nil,
    );
    let abort = typed(
        span,
        ExprNode::Send { recv: Some(base.clone()), method: Symbol::from("push_bulk_abort"), args: vec![], block: None, parenthesized: false },
        Ty::Nil,
    );
    // The last batch is pushed only when every item was built, as
    // sidekiq-bulk maps a whole slice before pushing it; whatever ends the
    // block, the batch is then closed, and a slice the block raised in is
    // dropped.
    let guarded = typed(
        span,
        ExprNode::BeginRescue {
            body: typed(span, ExprNode::Seq { exprs: vec![body, end] }, Ty::Nil),
            rescues: Vec::new(),
            else_branch: None,
            ensure: Some(abort),
            implicit: false,
        },
        Ty::Nil,
    );
    folds.workers.insert(name.clone());
    folds.bulk.insert(name);
    Some(typed(span, ExprNode::Seq { exprs: vec![begin, guarded] }, Ty::Nil))
}

/// Give every read of the block parameter `name` the type `ty`, where
/// the analyzer left it with no concrete type.
fn retype_var(e: &mut Expr, name: &Symbol, ty: &Ty) {
    if let ExprNode::Var { name: n, .. } = &*e.node
        && n == name
        && e.ty.as_ref().is_none_or(|t| codec_of(t).is_err())
    {
        e.ty = Some(ty.clone());
    }
    e.node.for_each_child_mut(&mut |c| retype_var(c, name, ty));
}

/// A bulk block's item as a record of `model`: the parameter itself, and
/// each `item.<column>` read the analyzer left untyped, typed from the
/// model's columns (`item.id`, `item.account_id`).
fn retype_item(e: &mut Expr, name: &Symbol, model: &str, models: &NativeModels) {
    let untyped = |ty: &Option<Ty>| ty.as_ref().is_none_or(|t| codec_of(t).is_err());
    match &*e.node {
        ExprNode::Var { name: n, .. } if n == name && untyped(&e.ty) => {
            e.ty = Some(Ty::Class { id: crate::ident::ClassId(Symbol::from(model)), args: vec![].into() });
        }
        ExprNode::Send { recv: Some(r), method, args, block: None, .. }
            if args.is_empty()
                && matches!(&*r.node, ExprNode::Var { name: n, .. } if n == name)
                && untyped(&e.ty) =>
        {
            if let Some(ty) = models.columns.get(model).and_then(|c| c.get(method.as_str())) {
                e.ty = Some(ty.clone());
            }
        }
        _ => {}
    }
    e.node.for_each_child_mut(&mut |c| retype_item(c, name, model, models));
}

/// `push_bulk_begin(limit)`, `push_bulk_end` and `push_bulk_abort`:
/// nothing on a target without the push, where the jobs between them run
/// inline; on the Ruby family [`apply_ruby`] makes them
/// `SidekiqPush.bulk_begin` / `bulk_end` / `bulk_abort`.
pub fn bulk_wrappers(async_: &MethodDef) -> Vec<MethodDef> {
    let span = async_.body.span;
    let nil = typed(span, ExprNode::Lit { value: Literal::Nil }, Ty::Nil);
    let mut begin = async_.clone();
    begin.name = Symbol::from("push_bulk_begin");
    begin.params = vec![crate::dialect::Param::positional(Symbol::from("limit"))];
    begin.body = nil.clone();
    begin.signature = Some(Ty::Fn {
        params: vec![crate::ty::Param { name: Symbol::from("limit"), ty: Ty::Int.into(), kind: crate::ty::ParamKind::Required }]
            .into(),
        block: None,
        ret: std::sync::Arc::new(Ty::Nil),
        effects: crate::effect::EffectSet::pure(),
    });
    let no_args = |name: &str| {
        let mut m = async_.clone();
        m.name = Symbol::from(name);
        m.params = Vec::new();
        m.body = nil.clone();
        m.signature = Some(Ty::Fn {
            params: Vec::new().into(),
            block: None,
            ret: std::sync::Arc::new(Ty::Nil),
            effects: crate::effect::EffectSet::pure(),
        });
        m
    };
    vec![begin, no_args("push_bulk_end"), no_args("push_bulk_abort")]
}

/// The folds' errors: a `set(...)` before a worker's enqueue that isn't
/// literal options, which can't be honored, and a bulk push of a shape
/// [`fold_bulk`] doesn't lower.
pub fn rejected_diagnostics(folds: &NativeFolds) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = folds
        .rejected
        .iter()
        .map(|span| {
            error(
                *span,
                "job-set-options",
                "not a literal option",
                "under queue_adapter :sidekiq a `set(...)` before a Sidekiq worker's `perform_async`, \
                 `perform_in` or `perform_at` must be literal `queue:`, `wait:`, `wait_until:` or \
                 `retry:` options; this one can't be honored, and the job would run early or on the \
                 wrong queue"
                    .to_string(),
            )
        })
        .collect();
    out.extend(folds.rejected_bulk.iter().map(|span| {
        error(
            *span,
            "job-bulk-push",
            "bulk push not lowered",
            "under queue_adapter :sidekiq a bulk push must be `W.push_bulk(items, limit: N) { |item| …; [args] }` \
             (or with no block), or `W.perform_bulk([[args], …], batch_size: N)` with a literal list and \
             literal `set(queue:, retry:)` options; this one isn't lowered, so its jobs would not be pushed"
                .to_string(),
        )
    }));
    out
}

/// `perform_async_with(at, queue, retry_json, shape, argc, *params)`:
/// `async_`'s body with its parameters moved `LEAD` places along. The
/// lead names are made fresh past the job's own.
pub fn perform_async_with(async_: &MethodDef) -> MethodDef {
    fn shift(e: &mut Expr) {
        if let ExprNode::Var { id, .. } = &mut *e.node {
            id.0 += LEAD as u32;
        }
        e.node.for_each_child_mut(&mut |c| shift(c));
    }
    let mut w = async_.clone();
    w.name = Symbol::from("perform_async_with");
    shift(&mut w.body);
    let names = lead_names(&async_.params);
    let tys = [Ty::Float, Ty::Str, Ty::Str, Ty::Int, Ty::Int];
    let mut params: Vec<crate::dialect::Param> =
        names.iter().map(|n| crate::dialect::Param::positional(Symbol::from(n.as_str()))).collect();
    params.extend(async_.params.iter().cloned());
    w.params = params;
    if let Some(Ty::Fn { params: own, block, ret, effects }) = &async_.signature {
        let mut all: Vec<crate::ty::Param> = names
            .iter()
            .zip(tys)
            .map(|(n, ty)| crate::ty::Param {
                name: Symbol::from(n.as_str()),
                ty: ty.into(),
                kind: crate::ty::ParamKind::Required,
            })
            .collect();
        all.extend(own.iter().cloned());
        w.signature = Some(Ty::Fn { params: all.into(), block: block.clone(), ret: ret.clone(), effects: effects.clone() });
    }
    w
}

/// `at`, `queue`, `retry_json`, `shape`, `argc`, each made fresh
/// past the job's own parameter names.
fn lead_names(own: &[crate::dialect::Param]) -> Vec<String> {
    let own: Vec<&str> = own.iter().map(|p| p.name.as_str()).collect();
    ["at", "queue", "retry_json", "shape", "argc"]
        .iter()
        .map(|b| super::job_payload::fresh_name(b, &own))
        .collect()
}

// ---- Planning ---------------------------------------------------------

/// One call site: where it is, and its argument types after the lead
/// (`None` for unknown).
struct Site {
    span: Span,
    args: Vec<Option<Ty>>,
}

fn collect_sites(
    e: &Expr,
    workers: &BTreeSet<String>,
    out: &mut BTreeMap<String, Vec<Site>>,
    unfolded: &mut BTreeSet<String>,
) {
    if let ExprNode::Send { recv: Some(r), method, args, .. } = &*e.node
        && let ExprNode::Const { path } = &*r.node
    {
        let name = path.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("::");
        let name = name.trim_start_matches("::").to_string();
        if workers.contains(&name) {
            match method.as_str() {
                "perform_async_with" => {
                    let site = Site { span: e.span, args: args.iter().skip(LEAD).map(|a| a.ty.clone()).collect() };
                    out.entry(name).or_default().push(site);
                }
                // Left standing by the fold: a splat, whose types aren't known.
                "perform_async" | "perform_in" | "perform_at" => {
                    unfolded.insert(name);
                }
                _ => {}
            }
        }
    }
    e.node.for_each_child(&mut |c| collect_sites(c, workers, out, unfolded));
}

/// An enqueue that doesn't name its worker (a bare `perform_async`, or
/// one on `self`) in a class's own methods: it can pass anything.
fn self_enqueues(e: &Expr) -> bool {
    fn walk(e: &Expr, hit: &mut bool) {
        if let ExprNode::Send { recv, method, .. } = &*e.node
            && matches!(method.as_str(), "perform_async" | "perform_in" | "perform_at" | "perform_async_with")
            && recv.as_ref().is_none_or(|r| matches!(&*r.node, ExprNode::SelfRef))
        {
            *hit = true;
        }
        e.node.for_each_child(&mut |c| walk(c, hit));
    }
    let mut hit = false;
    walk(e, &mut hit);
    hit
}

/// A plan for every worker in `workers`, from the folded call sites and
/// the options read before `job_class_side` stripped them (`options`).
pub fn plan_workers(
    app: &App,
    workers: &BTreeSet<String>,
    options: &BTreeMap<String, Vec<(String, String)>>,
) -> Vec<NativePlan> {
    let mut sites: BTreeMap<String, Vec<Site>> = BTreeMap::new();
    let mut unfolded: BTreeSet<String> = BTreeSet::new();
    crate::lower::for_each_hook_body_ref(app, &mut |body| collect_sites(body, workers, &mut sites, &mut unfolded));
    let mut plans = Vec::new();
    for name in workers {
        let Some(lc) = app.library_classes.iter().find(|lc| lc.name.0.as_str() == name) else { continue };
        let Some(perform) = inherited_perform(app, lc) else { continue };
        let opts = options.get(name).cloned().unwrap_or_default();
        let retry = opts.iter().find(|(k, _)| k == "retry").map_or_else(|| "true".to_string(), |(_, v)| v.clone());
        let queue = opts
            .iter()
            .find(|(k, _)| k == "queue")
            .and_then(|(_, v)| serde_json::from_str::<String>(v).ok())
            .unwrap_or_else(|| "default".to_string());
        let extra: String = opts
            .iter()
            .filter(|(k, _)| k != "retry" && k != "queue")
            .map(|(k, v)| format!(",{}:{v}", json_str(k)))
            .collect();
        let self_called = ancestry(app, lc).iter().any(|c| {
            c.methods.iter().filter(|m| m.name.as_str() != "perform_async_with").any(|m| self_enqueues(&m.body))
        });
        let planned = if self_called {
            Err(("an enqueue in the worker's own methods doesn't name its class, so its argument types are unknown".to_string(), None))
        } else if unfolded.contains(name) {
            Err(("an enqueue splats its arguments, so their types are unknown".to_string(), None))
        } else {
            plan_params(&perform.params, sites.get(name).map(Vec::as_slice).unwrap_or(&[]))
        };
        let (params, fallback, fallback_site) = match planned {
            Ok(p) => (p, None, None),
            Err((reason, site)) => (Vec::new(), Some(reason), site),
        };
        plans.push(NativePlan { worker: name.clone(), retry, queue, extra, params, fallback, fallback_site });
    }
    plans
}

/// The fallback of a worker no call site enqueues. Nothing pushes it, so
/// keeping its Proc is a ledger line, not an error.
const NO_SITES: &str = "no call site enqueues it, so its argument types are unknown";

/// The codecs for `perform`'s parameters, or why there are none and the
/// call site that shows it, when one does.
fn plan_params(params: &[crate::dialect::Param], sites: &[Site]) -> Result<Vec<NativeParam>, (String, Option<Span>)> {
    if params.iter().any(|p| p.rest || p.keyword || p.forwarding) {
        return Err(("perform takes a rest, keyword or forwarding parameter".to_string(), None));
    }
    if sites.is_empty() && !params.is_empty() {
        return Err((NO_SITES.to_string(), None));
    }
    let required = params.iter().filter(|p| p.default.is_none()).count();
    for site in sites {
        let n = site.args.len();
        if n < required {
            return Err((format!("a call site passes {n} arguments; perform needs {required}"), Some(site.span)));
        }
        if n > params.len() {
            return Err((format!("a call site passes {n} arguments; perform takes {}", params.len()), Some(site.span)));
        }
    }
    let mut out = Vec::new();
    for (i, p) in params.iter().enumerate() {
        let mut codec: Option<NativeCodec> = None;
        for site in sites {
            let Some(slot) = site.args.get(i) else { continue };
            let at = Some(site.span);
            let Some(ty) = slot else {
                return Err((format!("`{}`: an argument's type is unknown", p.name.as_str()), at));
            };
            let c = codec_of(ty).map_err(|what| {
                (
                    format!(
                        "`{}` is {what}; Sidekiq arguments must be native JSON (a String, Integer, Float, true, false, nil, Array, or Hash with String keys)",
                        p.name.as_str()
                    ),
                    at,
                )
            })?;
            codec = Some(match codec {
                None => c,
                Some(prev) => join(prev, c).ok_or_else(|| (format!("`{}`: call sites disagree on its type", p.name.as_str()), at))?,
            });
        }
        // A position some site passes, with a default: the default's type
        // joins in, so the writer types `perform`'s parameter as it is
        // (`flag = nil` beside sites passing Integers is an Integer or nil).
        // A position no site passes is never written, whatever its default.
        if let (Some(c), Some(default)) = (codec.clone(), p.default.as_ref())
            && let Ok(d) = default_codec(default)
        {
            codec = Some(join(c, d).ok_or_else(|| {
                (format!("`{}`: its default and its call sites disagree on its type", p.name.as_str()), None)
            })?);
        }
        let codec = codec.unwrap_or(NativeCodec::Omitted);
        if contains_unknown(&codec) {
            return Err((format!("`{}`: the type of an empty Array's or Hash's elements is unknown", p.name.as_str()), None));
        }
        out.push(NativeParam { name: p.name.clone(), codec });
    }
    Ok(out)
}

/// The codec for one call site's type, or what the type is when Sidekiq
/// would refuse it.
fn codec_of(ty: &Ty) -> Result<NativeCodec, String> {
    Ok(match ty {
        Ty::Int => NativeCodec::Int,
        Ty::Float => NativeCodec::Float,
        Ty::Str => NativeCodec::Str,
        Ty::Bool => NativeCodec::Bool,
        Ty::Nil => NativeCodec::Null,
        Ty::Sym => return Err("a Symbol".to_string()),
        Ty::Time | Ty::Date => return Err("a Time".to_string()),
        Ty::Array { elem } => NativeCodec::Array(Box::new(element(elem)?)),
        Ty::Hash { key, value } => {
            match &**key {
                Ty::Str | Ty::Untyped => {}
                Ty::Sym => return Err("a Hash with Symbol keys".to_string()),
                _ => return Err("a Hash whose keys aren't Strings".to_string()),
            }
            NativeCodec::Hash(Box::new(element(value)?))
        }
        Ty::Union { variants } => {
            let mut acc: Option<NativeCodec> = None;
            for v in variants.iter() {
                let c = codec_of(v)?;
                acc = Some(match acc {
                    None => c,
                    Some(prev) => join(prev, c).ok_or_else(|| {
                        let names: Vec<String> = variants.iter().map(type_name).collect();
                        format!("a value of several types ({})", names.join(" or "))
                    })?,
                });
            }
            acc.ok_or_else(|| "of no type".to_string())?
        }
        Ty::Untyped => return Err("of unknown type".to_string()),
        Ty::Class { id, .. } => return Err(format!("a {}", id.0.as_str())),
        other => return Err(type_name(other)),
    })
}

/// A type as the diagnostics name it.
fn type_name(ty: &Ty) -> String {
    match ty {
        Ty::Int => "an Integer".to_string(),
        Ty::Float => "a Float".to_string(),
        Ty::Str => "a String".to_string(),
        Ty::Bool => "a Boolean".to_string(),
        Ty::Nil => "nil".to_string(),
        Ty::Sym => "a Symbol".to_string(),
        Ty::Time | Ty::Date => "a Time".to_string(),
        Ty::Array { .. } => "an Array".to_string(),
        Ty::Hash { .. } => "a Hash".to_string(),
        Ty::Relation { of } => format!("a relation of {}", of.0.as_str()),
        Ty::Record { .. } => "a record row".to_string(),
        Ty::Tuple { .. } => "a tuple".to_string(),
        Ty::Class { id, .. } => format!("a {}", id.0.as_str()),
        Ty::Var { .. } => "of a type not inferred".to_string(),
        Ty::Untyped => "of unknown type".to_string(),
        Ty::Fn { .. } => "a block".to_string(),
        Ty::Union { .. } => "a union".to_string(),
        _ => "not a JSON type".to_string(),
    }
}

/// A default's codec: from its type when the analyzer gave one, else from
/// the literal itself (`{}` and `[]` have elements of unknown type).
fn default_codec(e: &Expr) -> Result<NativeCodec, String> {
    if let Some(ty) = &e.ty
        && !matches!(ty, Ty::Untyped)
    {
        return codec_of(ty);
    }
    match &*e.node {
        ExprNode::Lit { value: Literal::Nil } => Ok(NativeCodec::Null),
        ExprNode::Lit { value: Literal::Bool { .. } } => Ok(NativeCodec::Bool),
        ExprNode::Lit { value: Literal::Int { .. } } => Ok(NativeCodec::Int),
        ExprNode::Lit { value: Literal::Float { .. } } => Ok(NativeCodec::Float),
        ExprNode::Lit { value: Literal::Str { .. } } => Ok(NativeCodec::Str),
        ExprNode::Array { elements, .. } if elements.is_empty() => Ok(NativeCodec::Array(Box::new(NativeCodec::Omitted))),
        ExprNode::Hash { entries, .. } if entries.is_empty() => Ok(NativeCodec::Hash(Box::new(NativeCodec::Omitted))),
        _ => Err("not a literal".to_string()),
    }
}

fn element(ty: &Ty) -> Result<NativeCodec, String> {
    if matches!(ty, Ty::Untyped) {
        return Ok(NativeCodec::Omitted);
    }
    codec_of(ty)
}

/// A codec every value of which is a JSON scalar.
fn is_scalar(c: &NativeCodec) -> bool {
    use NativeCodec::*;
    match c {
        Bool | Int | Float | Str | Null | Scalar => true,
        Nullable(inner) => is_scalar(inner),
        _ => false,
    }
}

fn join(a: NativeCodec, b: NativeCodec) -> Option<NativeCodec> {
    use NativeCodec::*;
    Some(match (a, b) {
        (x, y) if x == y => x,
        (Omitted, x) | (x, Omitted) => x,
        // `Scalar` already holds nil.
        (Null, Scalar) | (Scalar, Null) => Scalar,
        // Different scalars: written by kind when the job is pushed.
        (x, y) if is_scalar(&x) && is_scalar(&y) && !matches!((&x, &y), (Null, _) | (_, Null)) => {
            match (x, y) {
                (Nullable(x), y) | (y, Nullable(x)) => match join(*x, y)? {
                    Scalar => Scalar,
                    j => Nullable(Box::new(j)),
                },
                _ => Scalar,
            }
        }
        (Null, Nullable(x)) | (Nullable(x), Null) => Nullable(x),
        (Null, x) | (x, Null) => Nullable(Box::new(x)),
        (Nullable(x), y) | (y, Nullable(x)) => Nullable(Box::new(join(*x, y)?)),
        (Array(x), Array(y)) => Array(Box::new(join(*x, *y)?)),
        (Hash(x), Hash(y)) => Hash(Box::new(join(*x, *y)?)),
        _ => return None,
    })
}

fn contains_unknown(c: &NativeCodec) -> bool {
    match c {
        NativeCodec::Array(inner) | NativeCodec::Hash(inner) => {
            matches!(**inner, NativeCodec::Omitted) || contains_unknown(inner)
        }
        NativeCodec::Nullable(inner) => contains_unknown(inner),
        _ => false,
    }
}

// ---- The Ruby-family rewrite ------------------------------------------

/// Rewrite each planned worker's enqueue arm to its push, and file each
/// worker that can't be pushed as an error: it would run in process
/// instead of where the app sends it.
pub fn apply_ruby(lcs: &mut [LibraryClass], app: &App) {
    if app.queue_adapter.as_deref() != Some("sidekiq") {
        return;
    }
    for lc in lcs.iter_mut() {
        let Some(plan) = app.native_plans.iter().find(|p| p.worker == lc.name.0.as_str()) else { continue };
        for m in lc.methods.iter_mut() {
            if m.receiver == MethodReceiver::Class
                && plan.is_payload()
                && matches!(m.name.as_str(), "push_bulk_begin" | "push_bulk_end" | "push_bulk_abort")
            {
                let span = m.body.span;
                let args = if m.name.as_str() == "push_bulk_begin" {
                    let mut limit = Expr::new(span, ExprNode::Var { id: crate::ident::VarId(0), name: Symbol::from("limit") });
                    limit.ty = Some(Ty::Int);
                    vec![limit]
                } else {
                    Vec::new()
                };
                let method = match m.name.as_str() {
                    "push_bulk_begin" => "bulk_begin",
                    "push_bulk_end" => "bulk_end",
                    _ => "bulk_abort",
                };
                m.body = typed(
                    span,
                    ExprNode::Send {
                        recv: Some(Expr::new(span, ExprNode::Const { path: vec![Symbol::from("SidekiqPush")] })),
                        method: Symbol::from(method),
                        args,
                        block: None,
                        parenthesized: true,
                    },
                    Ty::Nil,
                );
                continue;
            }
            if m.receiver != MethodReceiver::Class || !matches!(m.name.as_str(), "perform_async" | "perform_async_with") {
                continue;
            }
            if let Some(reason) = &plan.fallback {
                if m.name.as_str() == "perform_async" {
                    let message = format!(
                        "sidekiq_native: `{}`: {reason}; under queue_adapter :sidekiq the job must be pushed to Redis in Sidekiq's envelope",
                        plan.worker
                    );
                    // At the call site that shows it, when one does.
                    let span = plan.fallback_site.unwrap_or(m.name_span);
                    let d = if reason == NO_SITES {
                        warning(span, "job-closure-fallback", "argument not serializable", message)
                    } else {
                        error(span, "job-closure-fallback", "argument not serializable", message)
                    };
                    crate::emit::diagnostics::push(d);
                }
                continue;
            }
            let span = m.body.span;
            let vars: Vec<Expr> = m
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let mut v = Expr::new(span, ExprNode::Var { id: crate::ident::VarId(i as u32), name: p.name.clone() });
                    v.ty = match &m.signature {
                        Some(Ty::Fn { params, .. }) => params.get(i).map(|tp| (*tp.ty).clone()),
                        _ => None,
                    };
                    v
                })
                .collect();
            // `push_native_<w>(queue, at, retry_json, shape, argc, args…)`.
            let args: Vec<Expr> = if m.name.as_str() == "perform_async_with" {
                let mut all: Vec<Expr> = vars[..LEAD].to_vec();
                all.swap(0, 1);
                all.extend(vars[LEAD..].iter().cloned());
                all
            } else {
                let mut all = vec![
                    str_lit(span, ""),
                    float_lit(span, 0.0),
                    str_lit(span, ""),
                    int_lit(span, SHAPE_SETTER),
                    int_lit(span, vars.len() as i64),
                ];
                all.extend(vars);
                all
            };
            if !rewrite_enqueue(&mut m.body, plan, args) {
                crate::emit::diagnostics::push(error(
                    m.name_span,
                    "job-closure-fallback",
                    "argument not serializable",
                    format!(
                        "sidekiq_native: `{}`: the generated {} is not the shape this pass rewrites",
                        plan.worker,
                        m.name.as_str()
                    ),
                ));
            }
        }
    }
}

/// Replace `ActiveJob.enqueue_on(<queue>, <lambda>)` inside the
/// `drain_registered` arm `job_class_side` writes.
fn rewrite_enqueue(body: &mut Expr, plan: &NativePlan, args: Vec<Expr>) -> bool {
    let ExprNode::Seq { exprs } = &mut *body.node else { return false };
    for e in exprs.iter_mut() {
        let ExprNode::If { then_branch, .. } = &mut *e.node else { continue };
        let ExprNode::If { cond, then_branch: enqueue, .. } = &mut *then_branch.node else { continue };
        if !is_active_job_call(cond, "drain_registered") || !is_active_job_call(enqueue, "enqueue_on") {
            continue;
        }
        let span = enqueue.span;
        let mut call = Expr::new(
            span,
            ExprNode::Send {
                recv: Some(Expr::new(span, ExprNode::Const { path: vec![Symbol::from("JobRegistry")] })),
                method: Symbol::from(format!("push_native_{}", plan.suffix()).as_str()),
                args,
                block: None,
                parenthesized: true,
            },
        );
        call.ty = Some(Ty::Nil);
        *enqueue = call;
        return true;
    }
    false
}

fn is_active_job_call(e: &Expr, name: &str) -> bool {
    matches!(&*e.node, ExprNode::Send { recv: Some(r), method, .. }
        if method.as_str() == name
            && matches!(&*r.node, ExprNode::Const { path } if path.len() == 1 && path[0].as_str() == "ActiveJob"))
}

// ---- The registry ------------------------------------------------------

/// Each pushable worker's `push_native_<worker>`:
///
/// ```ruby
/// def self.push_native_ok_worker(queue, at, retry_json, shape, argc, id, opts)
///   SidekiqPush.enqueue_native("OkWorker", retry_json == "" ? "5" : retry_json,
///     queue == "" ? "pull" : queue, "", at, shape,
///     ActiveJob::Arguments.list([ActiveJob::Arguments.int(id), …].first(argc)))
/// end
/// ```
pub fn registry_source(plans: &[NativePlan]) -> String {
    let mut s = String::new();
    for plan in plans.iter().filter(|p| p.is_payload()) {
        let own: Vec<&str> = plan.params.iter().map(|p| p.name.as_str()).collect();
        let lead: Vec<String> = ["queue", "at", "retry_json", "shape", "argc"]
            .iter()
            .map(|b| super::job_payload::fresh_name(b, &own))
            .collect();
        let (queue, at, retry, shape, argc) = (&lead[0], &lead[1], &lead[2], &lead[3], &lead[4]);
        let mut params = lead.clone();
        params.extend(own.iter().map(|n| n.to_string()));
        let list = if plan.params.is_empty() {
            "\"[]\"".to_string()
        } else {
            let parts: Vec<String> = plan.params.iter().map(|p| write_expr(&p.codec, p.name.as_str(), 0)).collect();
            format!("ActiveJob::Arguments.list([{}].first({argc}))", parts.join(", "))
        };
        s.push_str(&format!("  def self.push_native_{}({})\n", plan.suffix(), params.join(", ")));
        s.push_str(&format!(
            "    SidekiqPush.enqueue_native({}, {retry} == \"\" ? {} : {retry}, {queue} == \"\" ? {} : {queue}, {}, {at}, {shape},\n      {list})\n",
            ruby_str(&plan.worker),
            ruby_str(&plan.retry),
            ruby_str(&plan.queue),
            ruby_str(&plan.extra),
        ));
        s.push_str("  end\n\n");
    }
    s
}

/// The JSON for `v` under `codec`, as Ruby text. `depth` keeps the block
/// variables of nested Arrays and Hashes apart.
fn write_expr(codec: &NativeCodec, v: &str, depth: usize) -> String {
    let a = "ActiveJob::Arguments";
    match codec {
        NativeCodec::Bool => format!("{a}.bool({v})"),
        NativeCodec::Int => format!("{a}.int({v})"),
        NativeCodec::Float => format!("{a}.float({v})"),
        NativeCodec::Str => format!("{a}.str({v})"),
        NativeCodec::Null => format!("{a}.null"),
        NativeCodec::Scalar => format!("SidekiqPush.native_value({v})"),
        NativeCodec::Omitted => "\"\"".to_string(),
        NativeCodec::Nullable(inner) => format!("({v}.nil? ? {a}.null : {})", write_expr(inner, v, depth)),
        NativeCodec::Array(inner) => {
            let e = format!("e{depth}");
            format!("{a}.list({v}.map {{ |{e}| {} }})", write_expr(inner, &e, depth + 1))
        }
        NativeCodec::Hash(inner) => {
            let (k, e) = (format!("k{depth}"), format!("e{depth}"));
            format!(
                "(\"{{\" + {v}.map {{ |{k}, {e}| {a}.str({k}) + \":\" + {} }}.join(\",\") + \"}}\")",
                write_expr(inner, &e, depth + 1)
            )
        }
    }
}

fn ruby_str(s: &str) -> String {
    crate::emit::ruby::expr::ruby_str_literal(s)
}
