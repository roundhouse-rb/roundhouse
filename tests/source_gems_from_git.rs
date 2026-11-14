//! `source_gems` reads a gem the lockfile takes from a `GIT` source where
//! Bundler checks it out: `<gem root>/bundler/gems/<repo>-<revision[..12]>`,
//! at the gemspec the source's glob finds, not `<gem root>/gems/<name>-<version>`.
use std::collections::HashMap;
use std::path::PathBuf;

use roundhouse::ingest::ingest_app_from_tree;

const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";
const CHECKOUT: &str = "vendor/bundle/ruby/3.4.0/bundler/gems/widgets-0123456789ab";

fn tree(glob: Option<&str>, gem_files: &[(&str, &str)]) -> HashMap<PathBuf, Vec<u8>> {
    let mut files = HashMap::<PathBuf, Vec<u8>>::new();
    let mut write = |path: &str, source: &str| {
        files.insert(path.into(), source.as_bytes().to_vec());
    };
    write(".bundle/config", "BUNDLE_PATH: vendor/bundle\n");
    write("roundhouse.yml", "extensions: [ext]\n");
    write("ext/gems.yml", "source_gems:\n  - {gem: acme-widgets}\n");
    let glob = glob.map(|g| format!("  glob: {g}\n")).unwrap_or_default();
    write(
        "Gemfile.lock",
        &format!(
            "GIT\n  remote: https://gitlab.example.com/acme/widgets.git\n  revision: {REVISION}\n{glob}  specs:\n    acme-widgets (1.2.0)\n\n\
             GEM\n  remote: https://rubygems.org/\n  specs:\n\n\
             DEPENDENCIES\n  acme-widgets!\n"
        ),
    );
    for (path, source) in gem_files {
        write(&format!("{CHECKOUT}/{path}"), source);
    }
    write("lib/probe.rb", "class Probe\n  def value\n    Acme::Widgets.source_marker\n  end\nend\n");
    files
}

const ENTRY: &str = "require_relative \"widgets/version\"\nmodule Acme\n  module Widgets\n    def self.source_marker\n      VERSION\n    end\n  end\nend\n";
const VERSION: &str = "module Acme\n  module Widgets\n    VERSION = \"1.2.0\"\n  end\nend\n";

fn assert_sourced(app: &roundhouse::App, lib: &str) {
    assert!(app.gem_lock.as_ref().unwrap().is_source("acme-widgets"), "acme-widgets is not a source gem");
    for file in ["acme-widgets.rb", "widgets/version.rb"] {
        let mounted = format!(".roundhouse/source_gems/acme-widgets-1.2.0/lib/{file}");
        assert!(
            app.sources.iter().any(|s| s.path.ends_with(&mounted)),
            "{mounted} not read from {lib}; sources: {:?}",
            app.sources.iter().map(|s| &s.path).collect::<Vec<_>>()
        );
    }
    assert!(app.library_classes.iter().any(|c| c.name.0.as_str() == "Acme::Widgets"), "Acme::Widgets not compiled");
}

#[test]
fn a_git_gem_is_read_from_its_bundler_checkout() {
    let app = ingest_app_from_tree(tree(
        None,
        &[
            ("acme-widgets.gemspec", "Gem::Specification.new { |s| s.name = \"acme-widgets\" }\n"),
            ("lib/acme-widgets.rb", ENTRY),
            ("lib/widgets/version.rb", VERSION),
        ],
    ))
    .expect("ingest");
    assert_sourced(&app, "the checkout's root");
}

#[test]
fn a_git_gem_in_a_subdirectory_is_read_where_its_gemspec_is() {
    // Bundler's default glob `{,*,*/*}.gemspec` finds a gemspec one directory down (a monorepo).
    let app = ingest_app_from_tree(tree(
        None,
        &[
            ("other/other.gemspec", "Gem::Specification.new { |s| s.name = \"other\" }\n"),
            ("other/lib/acme-widgets.rb", "raise \"wrong gem\"\n"),
            ("gem/acme-widgets.gemspec", "Gem::Specification.new { |s| s.name = \"acme-widgets\" }\n"),
            ("gem/lib/acme-widgets.rb", ENTRY),
            ("gem/lib/widgets/version.rb", VERSION),
        ],
    ))
    .expect("ingest");
    assert_sourced(&app, "gem/");
}

#[test]
fn a_git_gem_follows_the_sources_glob() {
    let app = ingest_app_from_tree(tree(
        Some("gems/*/*.gemspec"),
        &[
            ("acme-widgets.gemspec", "Gem::Specification.new { |s| s.name = \"acme-widgets\" }\n"),
            ("lib/acme-widgets.rb", "raise \"outside the glob\"\n"),
            ("gems/widgets/acme-widgets.gemspec", "Gem::Specification.new { |s| s.name = \"acme-widgets\" }\n"),
            ("gems/widgets/lib/acme-widgets.rb", ENTRY),
            ("gems/widgets/lib/widgets/version.rb", VERSION),
        ],
    ))
    .expect("ingest");
    assert_sourced(&app, "gems/widgets/");
}

#[test]
fn a_git_gem_without_its_checkout_names_the_checkout() {
    let err = ingest_app_from_tree(tree(None, &[])).expect_err("ingest without the checkout");
    let message = err.to_string();
    assert!(message.contains("bundler/gems/widgets-0123456789ab"), "{message}");
    assert!(message.contains("acme-widgets"), "{message}");
}
