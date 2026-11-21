//! An app file at the autoload path of a constant a source gem already
//! defines is a file Rails never loads.
//!
//! Zeitwerk sets an autoload for `Shelf::Box` from `app/models/shelf/box.rb`
//! only while `Shelf::Box` is undefined, and eager loading skips a file
//! whose constant is already defined the same way. A gem `Bundler.require`
//! loads defines its constants before the app's autoloaders are set up, so
//! the app's file at that path is never read, with eager loading or
//! without. A file the boot loads by path (`require Rails.root.join(…)`,
//! `require_relative`, from a boot file or from a file one of those loads)
//! is still loaded by that `require`, so it stays.
//!
//! The file is hidden from the ingest's tree ([`HiddenVfs`]) rather than
//! skipped by each pass, so no pass reads it.

use std::path::{Component, Path, PathBuf};

use ruby_prism::Node;

use crate::det::HashSet;
use crate::vfs::Vfs;

use super::source_gems::SourceGem;
use super::util::{class_name_path, constant_id_str, constant_path_segments_strs, module_name_path, string_value};

/// The app files under an autoload root whose constant, as Zeitwerk names
/// it from the file's path with the app's inflections, one of `gems`'
/// `lib/` files defines, of those the gem loads while it boots. Read before anything is hidden, so `vfs` is the
/// whole tree.
pub(super) fn shadowed<V: Vfs + ?Sized>(vfs: &V, dir: &Path, roots: &[PathBuf], gems: &[SourceGem]) -> HashSet<PathBuf> {
    let mut out = HashSet::default();
    if gems.is_empty() {
        return out;
    }
    let mut gem_files = Vec::new();
    for gem in gems {
        let lib = gem.mount.join("lib");
        if !vfs.is_dir(&lib) {
            continue;
        }
        // Not a generator: `rails generate` loads it, the app's boot does not.
        // Nor a file only a feature-name autoload reaches (`autoload :Box, "shelf/box"`): Zeitwerk sets its own autoload over that one.
        gem_files.extend(
            super::app::read_rb_files(vfs, &lib)
                .unwrap_or_default()
                .into_iter()
                .filter(|f| !f.strip_prefix(&lib).is_ok_and(|rel| rel.starts_with("generators")) && gem.loaded_at_boot(f)),
        );
    }
    let defined = defined_constants(vfs, &gem_files);
    if defined.is_empty() {
        return out;
    }
    let in_gem = |path: &Path| gems.iter().any(|gem| path.starts_with(&gem.mount));
    let mut autoload_roots: Vec<PathBuf> = Vec::new();
    for root in roots {
        let root = dir.join(root);
        if in_gem(&root) {
            continue;
        }
        for entry in vfs.read_dir(&root).unwrap_or_default() {
            if !vfs.is_dir(&entry) {
                continue;
            }
            // `app/models/concerns` is a root of its own, as Rails adds it.
            let concerns = entry.join("concerns");
            if vfs.is_dir(&concerns) {
                autoload_roots.push(concerns);
            }
            autoload_roots.push(entry);
        }
    }
    if let Ok(source) = vfs.read(&dir.join("config/application.rb")) {
        autoload_roots.extend(super::app::extract_autoload_path_roots(&source).into_iter().map(|root| dir.join(root)));
    }
    autoload_roots.retain(|root| !in_gem(root) && vfs.is_dir(root));
    autoload_roots.sort();
    autoload_roots.dedup();
    let mut loaded: Option<HashSet<PathBuf>> = None;
    let mut seen: HashSet<PathBuf> = HashSet::default();
    for root in &autoload_roots {
        for file in super::app::read_rb_files(vfs, root).unwrap_or_default() {
            if !seen.insert(file.clone()) {
                continue;
            }
            // The innermost root names it: `app/models/concerns/taggable.rb` is `Taggable`.
            let Some(rel) = autoload_roots
                .iter()
                .filter_map(|r| file.strip_prefix(r).ok())
                .min_by_key(|rel| rel.components().count())
                .and_then(|rel| rel.to_str())
                .and_then(|rel| rel.strip_suffix(".rb"))
            else {
                continue;
            };
            if !defined.contains(&crate::naming::camelize_path(rel)) {
                continue;
            }
            // Only walked when some file is a candidate.
            if loaded.get_or_insert_with(|| boot_loaded(vfs, dir)).contains(&normalized(&file)) {
                continue;
            }
            out.insert(file);
        }
    }
    out
}

/// What `files` define: each `class` and `module` declaration, and each
/// constant they assign (`Box = Class.new`, `Shelf::Box ||= …`), by its
/// full name.
fn defined_constants<V: Vfs + ?Sized>(vfs: &V, files: &[PathBuf]) -> HashSet<String> {
    let mut defined = Defined { scope: Vec::new(), out: HashSet::default() };
    for file in files {
        let Ok(bytes) = vfs.read(file) else { continue };
        let parsed = super::prism::parse_silent(&bytes);
        ruby_prism::Visit::visit(&mut defined, &parsed.node());
    }
    defined.out
}

struct Defined {
    /// The `class` and `module` bodies the visit is in, outermost first, as written.
    scope: Vec<String>,
    out: HashSet<String>,
}

impl Defined {
    fn record(&mut self, path: &[String]) {
        self.out.insert(self.scope.iter().chain(path).cloned().collect::<Vec<_>>().join("::"));
    }

    /// `::A::B` names `A::B` wherever it is written.
    fn record_path(&mut self, target: &Node<'_>) {
        let Some(path) = constant_path_segments_strs(target) else { return };
        if rooted(target) {
            self.out.insert(path.join("::"));
        } else {
            self.record(&path);
        }
    }

    /// A `class` or `module` body named `path`: recorded, and visited with
    /// its own name as the scope, which a rooted name (`class ::A::B`)
    /// starts afresh.
    fn declare(&mut self, name: &Node<'_>, path: Vec<String>, visit: impl FnOnce(&mut Self)) {
        let outer = if rooted(name) { std::mem::take(&mut self.scope) } else { self.scope.clone() };
        let depth = self.scope.len();
        self.record(&path);
        self.scope.extend(path);
        visit(self);
        self.scope.truncate(depth);
        if rooted(name) {
            self.scope = outer;
        }
    }
}

/// Whether a constant path starts at the top level (`::A::B`).
fn rooted(target: &Node<'_>) -> bool {
    let Some(mut node) = target.as_constant_path_node() else { return false };
    loop {
        match node.parent() {
            None => return true,
            Some(parent) => match parent.as_constant_path_node() {
                Some(next) => node = next,
                None => return false,
            },
        }
    }
}

impl<'pr> ruby_prism::Visit<'pr> for Defined {
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        let path = class_name_path(node).unwrap_or_default();
        self.declare(&node.constant_path(), path, |this| ruby_prism::visit_class_node(this, node));
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        let path = module_name_path(node).unwrap_or_default();
        self.declare(&node.constant_path(), path, |this| ruby_prism::visit_module_node(this, node));
    }

    fn visit_constant_write_node(&mut self, node: &ruby_prism::ConstantWriteNode<'pr>) {
        self.record(&[constant_id_str(&node.name()).to_string()]);
        ruby_prism::visit_constant_write_node(self, node);
    }

    fn visit_constant_or_write_node(&mut self, node: &ruby_prism::ConstantOrWriteNode<'pr>) {
        self.record(&[constant_id_str(&node.name()).to_string()]);
        ruby_prism::visit_constant_or_write_node(self, node);
    }

    fn visit_constant_path_write_node(&mut self, node: &ruby_prism::ConstantPathWriteNode<'pr>) {
        self.record_path(&node.target().as_node());
        ruby_prism::visit_constant_path_write_node(self, node);
    }

    fn visit_constant_path_or_write_node(&mut self, node: &ruby_prism::ConstantPathOrWriteNode<'pr>) {
        self.record_path(&node.target().as_node());
        ruby_prism::visit_constant_path_or_write_node(self, node);
    }
}

/// The app files the boot loads by path: what `config/application.rb`,
/// `environment.rb`, `boot.rb`, the environment files and the initializers
/// `require`, `require_relative` or `load`, and what those files load in
/// turn, each normalized. Parsed, so a require in a comment loads nothing.
fn boot_loaded<V: Vfs + ?Sized>(vfs: &V, dir: &Path) -> HashSet<PathBuf> {
    let config = dir.join("config");
    let mut pending: Vec<PathBuf> = ["application.rb", "environment.rb", "boot.rb"].iter().map(|f| config.join(f)).collect();
    for sub in ["environments", "initializers"] {
        let sub = config.join(sub);
        if vfs.is_dir(&sub) {
            pending.extend(super::app::read_rb_files(vfs, &sub).unwrap_or_default());
        }
    }
    let mut visited: HashSet<PathBuf> = pending.iter().map(|f| normalized(f)).collect();
    let mut loaded: HashSet<PathBuf> = HashSet::default();
    while let Some(file) = pending.pop() {
        let Ok(bytes) = vfs.read(&file) else { continue };
        let parsed = super::prism::parse_silent(&bytes);
        let mut requires = Requires { dir, base: file.parent().unwrap_or(dir).to_path_buf(), targets: Vec::new() };
        ruby_prism::Visit::visit(&mut requires, &parsed.node());
        for mut target in requires.targets {
            if target.extension().is_none() {
                target.set_extension("rb");
            }
            let target = normalized(&target);
            if !vfs.exists(&target) || vfs.is_dir(&target) {
                continue;
            }
            loaded.insert(target.clone());
            if visited.insert(target.clone()) {
                pending.push(target);
            }
        }
    }
    loaded
}

/// The paths a file's `require`, `require_relative`, `require_dependency`
/// and `load` calls name, where the argument is a path written out: a
/// string for `require_relative`, `Rails.root.join(…)`, `"#{Rails.root}/…"`,
/// `File.expand_path(…, __dir__)` or `File.join(__dir__, …)`, each with or
/// without `.to_s`.
struct Requires<'a> {
    dir: &'a Path,
    base: PathBuf,
    targets: Vec<PathBuf>,
}

impl Requires<'_> {
    fn path(&self, node: &Node<'_>) -> Option<PathBuf> {
        if let Some(call) = node.as_call_node() {
            let name = constant_id_str(&call.name());
            let args: Vec<Node<'_>> = call.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
            let receiver = call.receiver();
            if name == "to_s" && args.is_empty() {
                return self.path(&receiver?);
            }
            let strings = |nodes: &[Node<'_>]| nodes.iter().map(string_value).collect::<Option<Vec<String>>>();
            let receiver_is = |text: &[u8]| receiver.as_ref().is_some_and(|r| r.location().as_slice() == text);
            if name == "join" && receiver_is(b"Rails.root") && !args.is_empty() {
                return Some(self.dir.join(strings(&args)?.join("/")));
            }
            let is_dir = |n: &Node<'_>| n.as_call_node().is_some_and(|c| c.receiver().is_none() && c.arguments().is_none() && constant_id_str(&c.name()) == "__dir__");
            if name == "expand_path" && receiver_is(b"File") && args.len() == 2 && is_dir(&args[1]) {
                return Some(self.base.join(string_value(&args[0])?));
            }
            if name == "join" && receiver_is(b"File") && args.len() >= 2 && is_dir(&args[0]) {
                return Some(self.base.join(strings(&args[1..])?.join("/")));
            }
            return None;
        }
        let interpolated = node.as_interpolated_string_node()?;
        let parts: Vec<Node<'_>> = interpolated.parts().iter().collect();
        let [head, tail] = parts.as_slice() else { return None };
        let statements: Vec<Node<'_>> = head.as_embedded_statements_node()?.statements()?.body().iter().collect();
        let [only] = statements.as_slice() else { return None };
        if only.location().as_slice() != b"Rails.root" {
            return None;
        }
        let tail = String::from_utf8_lossy(tail.as_string_node()?.unescaped()).into_owned();
        Some(self.dir.join(tail.strip_prefix('/')?))
    }
}

impl<'pr> ruby_prism::Visit<'pr> for Requires<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let name = constant_id_str(&node.name());
        if node.receiver().is_none() && matches!(name, "require" | "require_relative" | "require_dependency" | "load") {
            let args: Vec<Node<'_>> = node.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
            if let Some(first) = args.first() {
                let target = match (name, string_value(first)) {
                    ("require_relative", Some(relative)) => Some(self.base.join(relative)),
                    (_, Some(written)) if Path::new(&written).is_absolute() => Some(PathBuf::from(written)),
                    // A bare feature goes through the load path, which this does not follow.
                    (_, Some(_)) => None,
                    (_, None) => self.path(first),
                };
                self.targets.extend(target);
            }
        }
        ruby_prism::visit_call_node(self, node);
    }
}

/// `path` with `.` dropped and `..` taken back, without the filesystem.
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// `inner` without the [`shadowed`] files.
pub(super) struct HiddenVfs<'a, V: Vfs + ?Sized> {
    pub(super) inner: &'a V,
    pub(super) hidden: HashSet<PathBuf>,
}

impl<V: Vfs + ?Sized> HiddenVfs<'_, V> {
    fn check(&self, path: &Path) -> std::io::Result<()> {
        if self.hidden.contains(path) {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "a source gem defines this file's constant, so Rails never loads it"))
        } else {
            Ok(())
        }
    }
}

impl<V: Vfs + ?Sized> Vfs for HiddenVfs<'_, V> {
    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        self.check(path)?;
        self.inner.read(path)
    }

    fn read_to_string(&self, path: &Path) -> std::io::Result<String> {
        self.check(path)?;
        self.inner.read_to_string(path)
    }

    fn read_dir(&self, path: &Path) -> std::io::Result<Vec<PathBuf>> {
        let mut entries = self.inner.read_dir(path)?;
        if !self.hidden.is_empty() {
            entries.retain(|entry| !self.hidden.contains(entry));
        }
        Ok(entries)
    }

    fn exists(&self, path: &Path) -> bool {
        !self.hidden.contains(path) && self.inner.exists(path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.inner.is_dir(path)
    }

    fn is_symlink(&self, path: &Path) -> bool {
        self.inner.is_symlink(path)
    }

    fn canonical(&self, path: &Path) -> Option<PathBuf> {
        self.inner.canonical(path)
    }
}

#[cfg(test)]
mod tests {
    use super::defined_constants;
    use crate::vfs::MapVfs;
    use std::path::PathBuf;

    fn defined(source: &str) -> Vec<String> {
        let file = PathBuf::from("/gem/lib/shelf.rb");
        let vfs = MapVfs::new([(file.clone(), source.as_bytes().to_vec())].into_iter().collect());
        let mut names: Vec<String> = defined_constants(&vfs, &[file]).into_iter().collect();
        names.sort();
        names
    }

    /// Ruby: `module Adapter; class ::Shelf::Box; end; end` defines `Shelf::Box`, and no `Adapter::Shelf`.
    #[test]
    fn a_rooted_class_inside_a_module_is_defined_at_the_top_level() {
        assert_eq!(defined("module Shelf\nend\n\nmodule Adapter\n  class ::Shelf::Box\n  end\nend\n"), ["Adapter", "Shelf", "Shelf::Box"]);
    }

    /// The body of `module ::Shelf` inside `module Adapter` is `Shelf`'s, so a class in it is `Shelf::Box`.
    #[test]
    fn a_rooted_module_s_body_is_scoped_at_the_top_level() {
        assert_eq!(defined("module Adapter\n  module ::Shelf\n    class Box\n    end\n  end\nend\n"), ["Adapter", "Shelf", "Shelf::Box"]);
    }

    #[test]
    fn a_nested_class_is_defined_under_its_modules() {
        assert_eq!(defined("module Adapter\n  class Shelf::Box\n  end\nend\n"), ["Adapter", "Adapter::Shelf::Box"]);
    }
}
