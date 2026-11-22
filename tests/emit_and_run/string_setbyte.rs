//! `String#setbyte` on the CRuby overlay, and its refusal on the strict
//! targets; the native twin is `spinel_toolchain/string_setbyte.rs`.

use super::string_setbyte_contract as contract;

/// The byte is written in place and the call answers it, as CRuby does.
#[test]
fn setbyte_writes_the_byte_in_place() {
    let run = contract::overlay().run_ruby(contract::SCRIPT);
    run.assert_passes();
    assert_eq!(run.stdout, contract::EXPECTED, "stderr:\n{}", run.stderr);
}

/// No strict target's runtime has `setbyte`. The analyzer types it (the
/// Ruby family's CSRF token XOR writes with it), so the shared emit gate
/// refuses it on a receiver that can be a String, and only there: an app
/// class's own `setbyte` is the app's, and the Ruby family runs both.
#[test]
fn strict_targets_refuse_setbyte_on_a_string() {
    use std::collections::{BTreeSet, HashMap};
    use std::path::PathBuf;

    use roundhouse::diagnostic::{DiagnosticKind, Severity};
    use roundhouse::project::{target_files, BuildTarget};

    // The contract's methods in a model, which every target emits, plus the
    // two classes a String's methods are also answered on: `Rails.env` and
    // a nested params value.
    // `shout` sends another String method by name: not a `setbyte`.
    let article = format!(
        "class Article < ApplicationRecord\n{}\n  def self.env_poke\n    Rails.env.setbyte(0, 65)\n  end\n\n  #: (String) -> String\n  def self.shout(s)\n    s.public_send(:upcase)\n  end\nend\n",
        contract::METHODS
    );
    let controller = "class ArticlesController < ApplicationController\n  def index\n    params[:a][:b].setbyte(0, 65)\n    head :ok\n  end\nend\n";
    let files = [
        ("db/schema.rb", "ActiveRecord::Schema.define(version: 1) do\n  create_table :articles do |t|\n    t.string :title\n  end\nend\n"),
        ("config/routes.rb", "Rails.application.routes.draw do\n  resources :articles, only: :index\nend\n"),
        ("app/models/application_record.rb", "class ApplicationRecord < ActiveRecord::Base\n  primary_abstract_class\nend\n"),
        ("app/models/article.rb", article.as_str()),
        ("app/models/register.rb", contract::REGISTER),
        ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
        ("app/controllers/articles_controller.rb", controller),
    ];
    // Every call in `source` spelled `call`, as the (start, end) of its span.
    let calls = |source: &str, call: &str| -> Vec<(u32, u32)> {
        source.match_indices(call).map(|(at, found)| (at as u32, (at + found.len()) as u32)).collect()
    };
    let mut strings: BTreeSet<(u32, u32)> = BTreeSet::new();
    strings.extend(calls(&article, "s.setbyte(i, b)"));
    strings.extend(calls(&article, "t.setbyte(i, b)"));
    strings.extend(calls(&article, "s.public_send(:setbyte, i, b)"));
    strings.extend(calls(&article, "Rails.env.setbyte(0, 65)"));
    strings.extend(calls(controller, "params[:a][:b].setbyte(0, 65)"));
    assert_eq!(strings.len(), 6, "`String`, `String?`, the copy, the sent call, `Rails.env`, the params value: {strings:?}");
    let own = calls(&article, "Register.new.setbyte(i, b)");
    assert_eq!(own.len(), 1);

    let tree: HashMap<PathBuf, Vec<u8>> = files.into_iter().map(|(p, c)| (p.into(), c.into())).collect();
    let mut app = roundhouse::ingest::ingest_app_from_tree(tree).expect("ingest");
    roundhouse::session::analyze_and_lower(&mut app);
    // Where `String#setbyte` is refused: one entry per call site, however
    // many gates name it (TypeScript's own String gate names it too).
    let refused = |diags: &[roundhouse::diagnostic::Diagnostic]| -> BTreeSet<(u32, u32)> {
        diags
            .iter()
            .filter(|d| {
                d.severity == Severity::Error
                    && matches!(&d.kind, DiagnosticKind::Unsupported { construct, .. } if construct.as_str() == "String#setbyte")
            })
            .map(|d| (d.span.start, d.span.end))
            .collect()
    };
    for target in [
        BuildTarget::Crystal,
        BuildTarget::Elixir,
        BuildTarget::Go,
        BuildTarget::Kotlin,
        BuildTarget::Python,
        BuildTarget::Rust,
        BuildTarget::Swift,
        BuildTarget::CSharp,
        BuildTarget::Typescript,
        BuildTarget::TypescriptWorker,
    ] {
        let (files, diags) = roundhouse::emit::diagnostics::scope(|| target_files(&app, std::path::Path::new("."), target));
        files.expect("target files");
        let sites = refused(&diags);
        // The six String sites and no other: not `Register#setbyte` at {own:?}.
        assert_eq!(sites, strings, "{target:?} refused String#setbyte at {sites:?}, not at the six String sites {strings:?} (Register's is {own:?}): {diags:?}");
    }
    for target in [BuildTarget::Ruby, BuildTarget::Jruby, BuildTarget::Spinel] {
        let (files, diags) = roundhouse::emit::diagnostics::scope(|| target_files(&app, std::path::Path::new("."), target));
        files.expect("target files");
        let sites = refused(&diags);
        assert!(sites.is_empty(), "{target:?} refused a setbyte it runs: {diags:?}");
    }
}
