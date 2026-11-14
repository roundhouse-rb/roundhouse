//! The locale files the app's gems put on `I18n.load_path`, as Rails
//! assembles it: Rails' own English, then each `Rails::Engine`'s
//! `config/locales/**/*.yml`, then the app's. A gem that is not an
//! engine adds its files from its own code, which no rule can read;
//! [`NON_ENGINE_LOCALES`] names those paths per gem.
//!
//! A gem is read where Bundler installed it: the app's `BUNDLE_PATH`
//! (`.bundle/config`), then `GEM_HOME` and `GEM_PATH`, at the version
//! `Gemfile.lock` resolved. A gem found nowhere contributes nothing, so
//! a key only it defines stays a missing translation.

use std::path::{Path, PathBuf};

use crate::gems::Lockfile;
use crate::vfs::Vfs;

/// Gems that are not engines but add locale files themselves, with the
/// files' paths under the gem's root (`config/locales/*.yml` style
/// globs: one `*` within a directory, or `**` for any depth).
pub const NON_ENGINE_LOCALES: &[(&str, &[&str])] = &[("rails-i18n", &["rails/locale/*.yml"])];

/// `(path, source)` for every gem locale file, in load order: in-repo
/// `PATH` gems, then installed gems, each in lockfile order; and each
/// gem of [`NON_ENGINE_LOCALES`] the lockfile names but no gem root has.
pub fn gem_locale_files<V: Vfs + ?Sized>(vfs: &V, app_dir: &Path, lock: &Lockfile, in_repo: &[PathBuf]) -> (Vec<(String, String)>, Vec<String>) {
    let roots = gem_roots(vfs, app_dir);
    let mut dirs: Vec<(String, PathBuf)> = in_repo.iter().map(|d| (d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), d.clone())).collect();
    let mut missing = Vec::new();
    for (name, version) in &lock.specs {
        if lock.is_in_repo(name) {
            continue;
        }
        match installed_gem(vfs, &roots, name, version) {
            Some(dir) => dirs.push((name.clone(), dir)),
            None if NON_ENGINE_LOCALES.iter().any(|(gem, _)| gem == name) => missing.push(format!("{name} {version}")),
            None => {}
        }
    }
    let mut files = Vec::new();
    for (name, dir) in dirs {
        let globs: Vec<&str> = match NON_ENGINE_LOCALES.iter().find(|(gem, _)| *gem == name) {
            Some((_, globs)) => globs.to_vec(),
            None if is_engine(vfs, &dir) => vec!["config/locales/**/*.yml"],
            None => continue,
        };
        for glob in globs {
            for path in expand(vfs, &dir, glob) {
                if let Ok(source) = vfs.read_to_string(&path) {
                    files.push((path.display().to_string(), source));
                }
            }
        }
    }
    (files, missing)
}

/// Where Bundler installs gems for this app, most specific first.
pub(crate) fn gem_roots<V: Vfs + ?Sized>(vfs: &V, app_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let configured = vfs
        .read_to_string(&app_dir.join(".bundle/config"))
        .ok()
        .and_then(|text| text.lines().find_map(|l| l.trim().strip_prefix("BUNDLE_PATH:").map(|v| v.trim().trim_matches('"').to_string())))
        .or_else(|| std::env::var("BUNDLE_PATH").ok());
    if let Some(path) = configured {
        let base = app_dir.join(path);
        // Bundler nests installs under `ruby/<abi version>/`.
        if let Ok(abis) = vfs.read_dir(&base.join("ruby")) {
            roots.extend(abis.into_iter().filter(|p| vfs.is_dir(p)));
        }
    }
    for var in ["GEM_HOME", "GEM_PATH"] {
        if let Ok(value) = std::env::var(var) {
            roots.extend(value.split(':').filter(|p| !p.is_empty()).map(PathBuf::from));
        }
    }
    roots
}

/// `<root>/gems/<name>-<version>`, or a platform build of it (`-x86_64-linux`).
pub(crate) fn installed_gem<V: Vfs + ?Sized>(vfs: &V, roots: &[PathBuf], name: &str, version: &str) -> Option<PathBuf> {
    let exact = format!("{name}-{version}");
    roots.iter().find_map(|root| {
        let gems = root.join("gems");
        let direct = gems.join(&exact);
        if vfs.is_dir(&direct) {
            return Some(direct);
        }
        let mut builds: Vec<PathBuf> = vfs
            .read_dir(&gems)
            .ok()?
            .into_iter()
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&format!("{exact}-"))) && vfs.is_dir(p))
            .collect();
        builds.sort();
        builds.into_iter().next()
    })
}

/// Bundler's gemspec glob for a `GIT` source whose lockfile entry names none.
const DEFAULT_GEMSPEC_GLOB: &str = "{,*,*/*}.gemspec";

/// A gem the lockfile takes from a `GIT` source, where Bundler checks it
/// out: `<root>/bundler/gems/<repo>-<revision[..12]>`, at the directory of
/// the gemspec the source's glob finds in it. Bundler loads every gemspec
/// the glob finds and takes the one whose spec is `name`; with no Ruby to
/// evaluate one, this takes the file named `<name>.gemspec`, the first in
/// path order.
pub(crate) fn installed_git_gem<V: Vfs + ?Sized>(vfs: &V, roots: &[PathBuf], source: &crate::gems::GitSource, name: &str) -> Option<PathBuf> {
    let gemspec = format!("{name}.gemspec");
    let glob = source.glob.as_deref().unwrap_or(DEFAULT_GEMSPEC_GLOB);
    roots.iter().find_map(|root| {
        let checkout = root.join("bundler").join("gems").join(source.checkout_dir());
        if !vfs.is_dir(&checkout) {
            return None;
        }
        let mut found: Vec<PathBuf> = Vec::new();
        for pattern in brace_alternatives(glob) {
            let segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
            glob_files(vfs, &checkout, &segments, &mut found);
        }
        found.retain(|path| path.file_name().is_some_and(|n| n == gemspec.as_str()));
        found.sort();
        found.into_iter().next().and_then(|path| path.parent().map(Path::to_path_buf))
    })
}

/// `{a,b}` alternatives expanded, as `Dir[]` does before it matches:
/// `{,*,*/*}.gemspec` is `.gemspec`, `*.gemspec` and `*/*.gemspec`.
fn brace_alternatives(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else { return vec![pattern.to_string()] };
    let mut depth = 0;
    let mut cuts = vec![open];
    let mut close = None;
    for (at, c) in pattern[open..].char_indices().map(|(i, c)| (open + i, c)) {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(at);
                    break;
                }
            }
            ',' if depth == 1 => cuts.push(at),
            _ => {}
        }
    }
    // Not a group: an unclosed `{` is a literal character, as it is to `Dir[]`.
    let Some(close) = close else { return vec![pattern.to_string()] };
    cuts.push(close);
    let (head, tail) = (&pattern[..open], &pattern[close + 1..]);
    cuts.windows(2)
        .flat_map(|w| brace_alternatives(&format!("{head}{}{tail}", &pattern[w[0] + 1..w[1]])))
        .collect()
}

/// The files under `at` whose relative path matches `segments`: `*` and
/// `?` within one segment, `**` for any number of directories. A
/// directory reached through a symbolic link is not entered: it could lead
/// outside the checkout.
fn glob_files<V: Vfs + ?Sized>(vfs: &V, at: &Path, segments: &[&str], out: &mut Vec<PathBuf>) {
    let Some((first, rest)) = segments.split_first() else { return };
    let Ok(entries) = vfs.read_dir(at) else { return };
    if *first == "**" {
        glob_files(vfs, at, rest, out);
        for entry in entries {
            let hidden = entry.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
            if !hidden && vfs.is_dir(&entry) && !vfs.is_symlink(&entry) {
                glob_files(vfs, &entry, segments, out);
            }
        }
        return;
    }
    for entry in entries {
        let Some(name) = entry.file_name().and_then(|n| n.to_str()) else { continue };
        if !segment_matches(first, name) {
            continue;
        }
        if rest.is_empty() {
            if !vfs.is_dir(&entry) {
                out.push(entry);
            }
        } else if vfs.is_dir(&entry) && !vfs.is_symlink(&entry) {
            glob_files(vfs, &entry, rest, out);
        }
    }
}

/// One path segment against one glob segment. As `Dir[]` without
/// `File::FNM_DOTMATCH`, a wildcard does not match a leading dot.
fn segment_matches(pattern: &str, name: &str) -> bool {
    fn matches(p: &[char], n: &[char]) -> bool {
        match p.split_first() {
            None => n.is_empty(),
            Some(('*', rest)) => (0..=n.len()).any(|i| matches(rest, &n[i..])),
            Some(('?', rest)) => !n.is_empty() && matches(rest, &n[1..]),
            Some((c, rest)) => n.first() == Some(c) && matches(rest, &n[1..]),
        }
    }
    if name.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    matches(&p, &n)
}

/// A gem that defines a `Rails::Engine` subclass under `lib/`.
fn is_engine<V: Vfs + ?Sized>(vfs: &V, gem: &Path) -> bool {
    expand(vfs, gem, "lib/**/*.rb").iter().any(|path| {
        vfs.read_to_string(path).is_ok_and(|text| {
            text.lines().any(|l| {
                let l = l.trim_start();
                l.starts_with("class ") && (l.contains("< Rails::Engine") || l.contains("< ::Rails::Engine"))
            })
        })
    })
}

/// The files under `base` matching `glob`, sorted as `Dir[]` returns
/// them: `dir/*.ext` or `dir/**/*.ext`.
fn expand<V: Vfs + ?Sized>(vfs: &V, base: &Path, glob: &str) -> Vec<PathBuf> {
    let (dir, recursive, ext) = match glob.split_once("/**/*.") {
        Some((dir, ext)) => (dir, true, ext),
        None => match glob.rsplit_once("/*.") {
            Some((dir, ext)) => (dir, false, ext),
            None => return Vec::new(),
        },
    };
    let mut out = Vec::new();
    let mut stack = vec![base.join(dir)];
    while let Some(current) = stack.pop() {
        let Ok(entries) = vfs.read_dir(&current) else { continue };
        for entry in entries {
            if vfs.is_dir(&entry) {
                if recursive {
                    stack.push(entry);
                }
            } else if entry.extension().and_then(|e| e.to_str()) == Some(ext) {
                out.push(entry);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, path: &str, text: &str) {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    #[test]
    fn gemspec_globs_expand_and_match_as_dir_does() {
        assert_eq!(brace_alternatives("{,*,*/*}.gemspec"), vec![".gemspec", "*.gemspec", "*/*.gemspec"]);
        assert_eq!(brace_alternatives("{a,b{c,d}}/x"), vec!["a/x", "bc/x", "bd/x"]);
        assert_eq!(brace_alternatives("odd{.gemspec"), vec!["odd{.gemspec"]);
        assert!(segment_matches("*.gemspec", "widgets.gemspec"));
        assert!(segment_matches("w?dgets.gemspec", "widgets.gemspec"));
        assert!(!segment_matches("*.gemspec", "widgets.gemspec.bak"));
        assert!(!segment_matches("*", ".git"));
    }

    #[test]
    fn an_installed_engine_contributes_its_locales_and_a_plain_gem_does_not() {
        let root = std::env::temp_dir().join(format!("rh_gem_locales_{}", std::process::id()));
        let app = root.join("app");
        write(&app, ".bundle/config", "---\nBUNDLE_PATH: \"vendor/bundle\"\n");
        let gems = "vendor/bundle/ruby/3.4.0/gems";
        write(&app, &format!("{gems}/admin-1.0/lib/admin/engine.rb"), "module Admin\n  class Engine < ::Rails::Engine\n  end\nend\n");
        write(&app, &format!("{gems}/admin-1.0/config/locales/en.yml"), "en:\n  admin:\n    edit: Edit\n");
        write(&app, &format!("{gems}/admin-1.0/config/locales/nested/fr.yml"), "fr:\n  admin:\n    edit: Modifier\n");
        write(&app, &format!("{gems}/plain-2.0/lib/plain.rb"), "module Plain\nend\n");
        write(&app, &format!("{gems}/plain-2.0/config/locales/en.yml"), "en:\n  plain: x\n");
        let lock = Lockfile::parse("GEM\n  remote: https://rubygems.org/\n  specs:\n    admin (1.0)\n    plain (2.0)\n    absent (3.0)\n\nDEPENDENCIES\n  admin\n  plain\n  absent\n");
        let (files, missing) = gem_locale_files(&crate::vfs::FsVfs::new(), &app, &lock, &[]);
        assert!(missing.is_empty(), "{missing:?}");
        let names: Vec<String> = files.iter().map(|(p, _)| p.rsplit("/gems/").next().unwrap().to_string()).collect();
        assert_eq!(names, vec!["admin-1.0/config/locales/en.yml", "admin-1.0/config/locales/nested/fr.yml"]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
