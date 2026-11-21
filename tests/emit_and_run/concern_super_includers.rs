//! A `super` into a concern that several models include. The concern's
//! method runs on each includer's own association there too: a module the
//! model includes after it, or the model's own def, reaches it through
//! `super`, and Ruby's lookup order still decides which definition runs
//! first. The probe's expected values are ActiveRecord 8.1.4.1's answers.

use super::emit_and_run;

fn app(booth: bool) -> emit_and_run::Overlay {
    app_holding(booth, r##"module TokenHolder
  extend ActiveSupport::Concern

  def issue_token!(name)
    tokens.create!(name: name)
  end
end
"##)
}

fn app_holding(booth: bool, holder: &str) -> emit_and_run::Overlay {
    let app = emit_and_run::empty_app()
        .write("app/models/application_record.rb", r##"class ApplicationRecord < ActiveRecord::Base
  self.abstract_class = true
end
"##)
        .write("app/models/token.rb", r##"class Token < ApplicationRecord
  belongs_to :bearer, polymorphic: true
end
"##)
        .write("app/models/concerns/token_holder.rb", holder)
        .write("app/models/concerns/audited.rb", r##"module Audited
  extend ActiveSupport::Concern

  def issue_token!(name)
    super("audited #{name}")
  end
end
"##)
        .write("app/models/concerns/plain.rb", r##"module Plain
  def issue_token!(name)
    tokens.create!(name: "plain #{name}")
  end
end
"##)
        .write("app/models/cart.rb", r##"class Cart < ApplicationRecord
  has_many :tokens, as: :bearer
  include TokenHolder
  include Audited
end
"##)
        .write("db/schema.rb", r##"ActiveRecord::Schema.define do
  create_table "carts", force: :cascade do |t|
    t.string "name"
  end
  create_table "booths", force: :cascade do |t|
    t.string "name"
  end
  create_table "kiosks", force: :cascade do |t|
    t.string "name"
  end
  create_table "stands", force: :cascade do |t|
    t.string "name"
  end
  create_table "tokens", force: :cascade do |t|
    t.string "bearer_type"
    t.integer "bearer_id"
    t.string "name"
  end
end
"##);
    if !booth {
        return app;
    }
    app.write("app/models/booth.rb", r##"class Booth < ApplicationRecord
  has_many :tokens, as: :bearer
  include TokenHolder
end
"##)
        .write("app/models/kiosk.rb", r##"class Kiosk < ApplicationRecord
  has_many :tokens, as: :bearer
  include TokenHolder

  def issue_token!(name)
    super("own #{name}")
  end
end
"##)
        .write("app/models/stand.rb", r##"class Stand < ApplicationRecord
  has_many :tokens, as: :bearer
  include TokenHolder
  include Plain
end
"##)
}

fn check(app: emit_and_run::Overlay, call: &str, rails: &str) {
    let script = format!(
        "got = {call}\nputs \"got #{{got.inspect}}\"\nraise \"differs from Rails: #{{got.inspect}}\" unless got == {rails}\nputs \"probe passed\"\n"
    );
    let run = app.run_ruby(&script);
    run.assert_passes();
    assert!(run.stdout.contains("probe passed"), "{}", run.stdout);
}

/// `Audited#issue_token!` runs first on a `Cart`, and its `super` reaches
/// `TokenHolder#issue_token!`, which creates the token on the `Cart`'s
/// own association even though `Booth` includes `TokenHolder` too.
#[test]
fn a_super_into_a_concern_several_models_include_runs_on_the_includer_s_association() {
    check(
        app(true),
        r#"[Cart.create!.issue_token!("a"), Booth.create!.issue_token!("b")].map { |t| [t.bearer_type, t.name] }"#,
        r#"[["Cart", "audited a"], ["Booth", "b"]]"#,
    );
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn a_super_into_a_concern_several_models_include_runs_on_the_includer_s_association_on_spinel() {
    let run = app(true).run_spinel_with_db(
        "got = [Cart.create!.issue_token!(\"a\").name, Booth.create!.issue_token!(\"b\").name]\nraise \"differs from Rails\" unless got == [\"audited a\", \"b\"]\nputs \"probe passed\"\n",
    );
    run.assert_passes();
    assert!(run.stdout.contains("probe passed"), "{}", run.stdout);
}

/// The model's own def wins over the concern's, and its `super` reaches
/// the concern's method on its own association.
#[test]
fn a_model_s_own_def_wins_and_its_super_reaches_the_shared_concern() {
    check(
        app(true),
        r#"[Kiosk.create!.issue_token!("k")].map { |t| [t.bearer_type, t.name] }"#,
        r#"[["Kiosk", "own k"]]"#,
    );
}

/// A module included after the shared concern, without `super`, wins.
#[test]
fn a_module_included_after_the_shared_concern_still_wins() {
    check(
        app(true),
        r#"[Stand.create!.issue_token!("s")].map { |t| [t.bearer_type, t.name] }"#,
        r#"[["Stand", "plain s"]]"#,
    );
}

/// With one includer, the concern's body is lowered against it, as before.
#[test]
fn a_super_into_a_concern_one_model_includes_is_unchanged() {
    check(
        app(false),
        r#"[Cart.create!.issue_token!("a")].map { |t| [t.bearer_type, t.name] }"#,
        r#"[["Cart", "audited a"]]"#,
    );
}

/// A reached definition that reads its own name keeps it: `__method__`
/// there is `:issue_token!`, as Rails answers.
#[test]
fn a_reached_definition_reading_its_own_name_keeps_it() {
    let holder = r##"module TokenHolder
  extend ActiveSupport::Concern

  def issue_token!(name)
    return __method__ if name == "audited probe"
    tokens.create!(name: name)
  end
end
"##;
    check(app_holding(true, holder), r#"Cart.create!.issue_token!("probe")"#, ":issue_token!");
}

/// A reached definition asking `self.block_given?` still sees the block
/// the caller passed, which Ruby's `super(...)` forwards: Rails answers
/// `:with_block`.
#[test]
fn a_reached_definition_asking_self_block_given_still_sees_the_block() {
    let holder = r##"module TokenHolder
  extend ActiveSupport::Concern

  def issue_token!(name)
    return :with_block if self.block_given?
    tokens.create!(name: name)
  end
end
"##;
    check(app_holding(true, holder), r#"Cart.create!.issue_token!("x") { }"#, ":with_block");
}
