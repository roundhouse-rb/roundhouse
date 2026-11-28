//! Locale-specific templates: `show.de.html.erb` beside `show.en.html.erb`
//! (and perhaps `show.html.erb`) are one template, `show`, whose text
//! Rails picks by `I18n.locale` each time it renders it.
//!
//! Rails' `PathParser` reads `name(.locale)(.format)(+variant).handler`:
//! the segment before the format is a locale when it looks like one
//! (an available locale, or `xx` / `xx-YY`), and a lone segment is the
//! format when it names a MIME type (`show.js.erb`, `show.md.erb`) and a
//! locale otherwise (`show.de.erb`). The lookup (`lookup_context.rb`'s
//! locale detail) tries `I18n.locale`, then `I18n.fallbacks[locale]`, then
//! `default_locale`. Active Support always loads `I18n.fallbacks`, so a
//! locale's parents (`de` for `de-AT`) are tried whether or not the app
//! sets `config.i18n.fallbacks`, which only adds its defaults after them
//! (unlike `t`, whose backend falls back only when it is set). A template
//! without a locale matches any of them after those do, and one with an
//! explicit format before one without (`TemplateDetails#sort_key_for`
//! sorts by format first). The same lookup finds actions'
//! templates, partials, collection partials, layouts and mailer
//! templates.
//!
//! Ingest names each file by its whole stem (`pages/show.de`), so
//! nothing rendering `show` found the variants. Here the variants of one
//! name and format become ONE view under that name, its body a `case
//! I18n.locale.to_s` choosing, per available locale, the template the
//! lookup above finds — decided now, from the app's I18n settings, so
//! every render site, the signature (the variants' ivars and locals
//! together) and every target see one ordinary template. When one
//! template answers every locale there is no `case` at all. A locale
//! Rails makes available that this build's locales lack (`de-CH` from a
//! gem) gets an arm too when the template it finds is not the default's.
//!
//! What is not modeled is an error, and the variants keep their stems:
//! a locale no template answers (Rails' missing template), a Jbuilder
//! template, variants in different view paths, different strict-locals
//! headers, I18n settings the choice depends on but ingest cannot read,
//! and a template named for a locale Rails makes available that this
//! build's locales lack (an unset `available_locales` takes gems' and
//! Rails' own locale files too). A template for a locale Rails never
//! makes available is never picked, there as here. (A request variant,
//! `show.html+phone.erb`, is not ingested at all.)

use crate::app::App;
use crate::det::HashMap;
use crate::dialect::View;
use crate::expr::{Arm, Expr, ExprNode, LValue, Literal, Pattern};
use crate::ident::Symbol;

/// Rails 8.1's registered MIME type symbols (`action_dispatch/http/
/// mime_types.rb`) and turbo-rails' `turbo_stream`: a lone segment that
/// is one of these is the template's format, not a locale.
const FORMATS: &[&str] = &[
    "html", "text", "js", "css", "ics", "csv", "vcf", "vtt", "md", "png", "jpeg", "gif", "bmp", "tiff", "svg",
    "webp", "mpeg", "mp3", "ogg", "m4a", "webm", "mp4", "otf", "ttf", "woff", "woff2", "xml", "rss", "atom",
    "yaml", "multipart_form", "url_encoded_form", "json", "pdf", "zip", "gzip", "turbo_stream",
];

/// One template of a locale group.
struct Member {
    index: usize,
    locale: Option<String>,
    /// The file names its format (`show.html.erb`), which Rails sorts
    /// before a file that does not (`show.erb`); `None` until a group
    /// with a locale asks its file.
    explicit_format: Option<bool>,
}

/// `implicit`: when `config.i18n.available_locales` is not set, the
/// locales Rails makes available from files other than the app's (its
/// gems', and its own `en`), which `app.i18n` does not list.
pub(super) fn merge(app: &mut App, implicit: Option<&[String]>) {
    let mut available = app.i18n.locales();
    available.extend(implicit.unwrap_or_default().iter().cloned());
    // Each view by the name, format and kind it answers to once its
    // locale is read off.
    let mut groups: Vec<((String, String, bool), Vec<Member>)> = Vec::new();
    let mut at: HashMap<(String, String, bool), usize> = HashMap::default();
    for (index, view) in app.views.iter().enumerate() {
        let (name, locale, format, explicit_format) = split(view, &available);
        let key = (name, format, view.jbuilder);
        let slot = *at.entry(key.clone()).or_insert_with(|| {
            groups.push((key, Vec::new()));
            groups.len() - 1
        });
        groups[slot].1.push(Member { index, locale, explicit_format });
    }
    let mut merged: HashMap<usize, View> = HashMap::default();
    let mut dropped: crate::det::HashSet<usize> = Default::default();
    for ((name, format, jbuilder), members) in &groups {
        let Some(first) = members.iter().find(|m| m.locale.is_some()) else { continue };
        let refuse = |detail: String| {
            super::prism::push(crate::diagnostic::Diagnostic::unsupported(
                app.views[first.index].body.span,
                None,
                "locale_template",
                format!("`{name}` ({format}) has locale-specific templates, which Rails picks by `I18n.locale`: {detail}"),
            ));
        };
        if *jbuilder {
            refuse("a Jbuilder template's locale variants are not modeled".to_string());
            continue;
        }
        match choose(app, members, implicit) {
            Ok(view) => {
                let keep = members[0].index;
                let mut view = view;
                view.name = Symbol::from(name.as_str());
                view.format = Symbol::from(format.as_str());
                merged.insert(keep, view);
                dropped.extend(members.iter().map(|m| m.index).filter(|&i| i != keep));
            }
            Err(detail) => refuse(detail),
        }
    }
    if merged.is_empty() {
        return;
    }
    let views = std::mem::take(&mut app.views);
    app.views = views
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !dropped.contains(i))
        .map(|(i, view)| merged.remove(&i).unwrap_or(view))
        .collect();
}

/// The view's name without its locale, the locale, the format, and
/// whether its file names the format, when the name tells. Ingest
/// spelled `show.de.html.erb` as `show.de` (html) and `show.de.erb` as
/// `show` (de).
fn split(view: &View, available: &[String]) -> (String, Option<String>, String, Option<bool>) {
    let name = view.name.as_str();
    let format = view.format.as_str();
    let is_locale = |s: &str| names_locale(s, available);
    if is_locale(format) && !FORMATS.contains(&format) {
        return (name.to_string(), Some(format.to_string()), "html".to_string(), Some(false));
    }
    let (dir, base) = name.rsplit_once('/').map_or(("", name), |(d, b)| (d, b));
    if let Some((stem, locale)) = base.rsplit_once('.')
        && !stem.is_empty()
        && is_locale(locale)
    {
        let stem = if dir.is_empty() { stem.to_string() } else { format!("{dir}/{stem}") };
        return (stem, Some(locale.to_string()), format.to_string(), Some(true));
    }
    (name.to_string(), None, format.to_string(), None)
}

/// A lone segment before the handler that is no format and may be a
/// locale (`show.de.erb`, `show.zh-Hans.erb`): letters, digits, `-` and
/// `_`. Which it is waits for the app's locales ([`names_locale`]).
pub(super) fn lone_segment(segment: &str) -> bool {
    !FORMATS.contains(&segment)
        && segment.starts_with(|c: char| c.is_ascii_alphabetic())
        && segment.split(['-', '_']).all(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_alphanumeric()))
}

/// Whether `PathParser` reads `segment` as a locale: an available one,
/// or one shaped like a locale.
pub(super) fn names_locale(segment: &str, available: &[String]) -> bool {
    available.iter().any(|l| l == segment) || locale_shaped(segment)
}

/// `[a-z]{2}(?:[-_][A-Z]{2})?`, the shape `PathParser` takes for a
/// locale besides the available ones.
fn locale_shaped(s: &str) -> bool {
    let b = s.as_bytes();
    let two = |x: &[u8], upper: bool| {
        x.len() == 2 && x.iter().all(|c| if upper { c.is_ascii_uppercase() } else { c.is_ascii_lowercase() })
    };
    match b.len() {
        2 => two(b, false),
        5 => two(&b[..2], false) && matches!(b[2], b'-' | b'_') && two(&b[3..], true),
        _ => false,
    }
}

/// Whether the template's file spells a format (`show.html.erb`), read
/// off the path its body was registered under.
fn names_format(view: &View) -> bool {
    let Some(path) = super::sources::path_of(view.body.span.file) else { return true };
    let file = path.rsplit(['/', '\\']).next().unwrap_or(&path);
    file.rsplit_once('.').is_some_and(|(stem, _handler)| stem.contains('.'))
}

/// The one view the group renders as: per locale the template Rails'
/// lookup finds, as a `case` on `I18n.locale` when they differ.
fn choose(app: &App, members: &[Member], implicit: Option<&[String]>) -> Result<View, String> {
    let catalog = &app.i18n;
    let locales = catalog.locales();
    // With the locales unknown, any of them may be one a template names,
    // or its child (`de-CH` for `de`), and would render it; only a group
    // whose locale templates are the default locale's answers every
    // locale alike.
    if let Some(why) = &catalog.available_unknown
        && let Some(l) = members.iter().filter_map(|m| m.locale.as_ref()).find(|l| **l != catalog.locale)
    {
        return Err(format!(
            "which locales `I18n.locale` can be is unknown (`config.i18n.available_locales`: {why}), so whether `{l}` renders is too"
        ));
    }
    // A locale Rails makes available that this build's locales lack
    // would render its template there.
    if let Some(l) = members
        .iter()
        .filter_map(|m| m.locale.as_ref())
        .find(|l| !locales.contains(l) && implicit.is_some_and(|i| i.contains(l)))
    {
        return Err(format!(
            "`config.i18n.available_locales` is not set, so Rails makes `{l}` available from locale files other than the app's (a gem's, or Rails' own `en`), which this build's locales lack"
        ));
    }
    if let Some(why) = &catalog.fallbacks.unknown {
        return Err(format!("`config.i18n.fallbacks` {why}"));
    }
    let views = &app.views;
    let dir_of = |m: &Member| {
        super::sources::path_of(views[m.index].body.span.file)
            .map(|p| p.rsplit_once(['/', '\\']).map_or(String::new(), |(d, _)| d.to_string()))
    };
    let first_dir = dir_of(&members[0]);
    if members.iter().any(|m| dir_of(m) != first_dir) {
        return Err("its templates are in different view paths, where Rails takes the first path with any match".into());
    }

    // Per locale, the member its lookup finds, the same in every
    // environment. A locale Rails makes available that this build's
    // locales lack (`de-CH` from a gem) finds its parent's template.
    let extra = implicit.unwrap_or_default().iter().filter(|l| !locales.contains(l));
    let mut chosen: Vec<(String, usize)> = Vec::new();
    for locale in locales.iter().chain(extra) {
        let mut pick: Option<Option<usize>> = None;
        for variant in catalog.variants(std::slice::from_ref(locale)) {
            let mut chain = crate::i18n::fallback_chain(locale, variant.fallbacks.as_deref().unwrap_or_default());
            chain.push(catalog.locale.clone());
            let mut seen = crate::det::HashSet::default();
            chain.retain(|l| seen.insert(l.clone()));
            let found = members
                .iter()
                .enumerate()
                .filter_map(|(i, m)| {
                    let at = match &m.locale {
                        Some(l) => chain.iter().position(|c| c == l)?,
                        None => chain.len(),
                    };
                    let explicit = m.explicit_format.unwrap_or_else(|| names_format(&views[m.index]));
                    Some(((!explicit, at), i))
                })
                .min()
                .map(|(_, i)| i);
            match pick {
                None => pick = Some(found),
                Some(earlier) if earlier != found => {
                    return Err(format!(
                        "for `{locale}` the template differs by environment (`config.i18n.fallbacks` in `config/environments/`)"
                    ));
                }
                Some(_) => {}
            }
        }
        match pick.flatten() {
            Some(i) => chosen.push((locale.clone(), i)),
            None => {
                return Err(format!(
                    "for `{locale}` Rails finds none of them and there is no template without a locale, so it raises a missing template"
                ));
            }
        }
    }

    let used: Vec<usize> = {
        let mut u: Vec<usize> = chosen.iter().map(|(_, i)| *i).collect();
        u.sort();
        u.dedup();
        u
    };
    let strict = &views[members[used[0]].index].strict_locals;
    if used.iter().any(|&i| views[members[i].index].strict_locals != *strict) {
        return Err("their `locals:` headers differ".into());
    }
    // The default locale is first; its template answers any other.
    let default = chosen[0].1;
    let base = &views[members[default].index];
    if used.len() == 1 {
        return Ok(base.clone());
    }
    let mut arms = Vec::new();
    for (locale, i) in chosen.iter().skip(1).filter(|(_, i)| *i != default) {
        arms.push(Arm {
            pattern: Pattern::Lit { value: Literal::Str { value: locale.clone() } },
            guard: None,
            body: inner(&views[members[*i].index])?,
        });
    }
    arms.push(Arm { pattern: Pattern::Wildcard, guard: None, body: inner(base)? });
    let span = base.body.span;
    let locale = Expr::new(span, ExprNode::Send {
        recv: Some(Expr::new(span, ExprNode::Const { path: vec![Symbol::from("I18n")] })),
        method: Symbol::from("locale"),
        args: Vec::new(),
        block: None,
        parenthesized: false,
    });
    let scrutinee = Expr::new(span, ExprNode::Send {
        recv: Some(locale),
        method: Symbol::from("to_s"),
        args: Vec::new(),
        block: None,
        parenthesized: false,
    });
    let ExprNode::Seq { exprs } = &*base.body.node else { unreachable!("inner checked the shape") };
    let body = vec![exprs[0].clone(), Expr::new(span, ExprNode::Case { scrutinee, arms }), exprs[exprs.len() - 1].clone()];
    Ok(View { body: Expr::new(span, ExprNode::Seq { exprs: body }), ..base.clone() })
}

/// A template's statements between its `_buf = ""` and its closing
/// `_buf`, the shape every text template compiles to.
fn inner(view: &View) -> Result<Expr, String> {
    let ExprNode::Seq { exprs } = &*view.body.node else { return Err(handler_refusal(view)) };
    let opens = exprs.first().is_some_and(|e| matches!(&*e.node,
        ExprNode::Assign { target: LValue::Var { name, .. }, value }
            if name.as_str() == "_buf" && matches!(&*value.node, ExprNode::Lit { value: Literal::Str { value } } if value.is_empty())));
    let closes = exprs.len() >= 2
        && exprs.last().is_some_and(|e| matches!(&*e.node, ExprNode::Var { name, .. } if name.as_str() == "_buf"));
    if !opens || !closes {
        return Err(handler_refusal(view));
    }
    let span = view.body.span;
    Ok(Expr::new(span, ExprNode::Seq { exprs: exprs[1..exprs.len() - 1].to_vec() }))
}

fn handler_refusal(view: &View) -> String {
    format!("`{}`'s handler does not render through a template buffer, which the choice between them needs", view.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::{locale_shaped, lone_segment};

    #[test]
    fn locale_shape_is_path_parsers() {
        for yes in ["de", "en", "pt-BR", "zh_TW", "js", "md"] {
            assert!(locale_shaped(yes), "{yes}");
        }
        for no in ["html", "DE", "d", "deu", "pt-br", "sr-Latn", "en-"] {
            assert!(!locale_shaped(no), "{no}");
        }
    }

    #[test]
    fn a_lone_segment_may_be_any_locale_but_no_format() {
        for yes in ["de", "zh-Hans", "es-419", "sr_Latn", "foo"] {
            assert!(lone_segment(yes), "{yes}");
        }
        for no in ["html", "js", "turbo_stream", "en-", "-en", "de--AT", "1x", "de.AT", "de+phone", ""] {
            assert!(!lone_segment(no), "{no}");
        }
    }
}
