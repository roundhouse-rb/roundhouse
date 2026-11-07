//! `errors.full_messages_for(:field)` and `errors.messages_for(:field)`
//! read one attribute's errors as Rails 8.1 does: the full messages of
//! `where(:field)`, and the messages `errors[:field]` answers, for a
//! Symbol or a String field, `[]` when the field has none. A model, a
//! view and a test read them alike.
#[path = "support/emit_and_run.rs"]
mod emit_and_run;

use roundhouse::project::BuildTarget;

fn messages_for_app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .edit(
            "app/models/article.rb",
            "  validates :title, presence: true\n",
            "  validates :title, presence: true
  validate :note_check

  def note_check
    errors.add(:base, \"is odd\") if body == \"an odd body\"
  end

  def title_problems
    errors.full_messages_for(:title).join(\", \")
  end

  def field_readers
    [errors.full_messages_for(\"body\"), errors.full_messages_for(:base), errors.full_messages_for(:comments),
     errors.messages_for(:title), errors.messages_for(\"body\")]
  end
",
        )
        .edit(
            "app/views/articles/_form.html.erb",
            "      <ul class=\"list-disc ml-6\">\n",
            "      <p id=\"title_problems\"><%= article.errors.full_messages_for(:title).to_sentence %></p>
      <p id=\"body_messages\"><% article.errors.messages_for(\"body\").each do |message| %>[<%= message %>]<% end %></p>
      <ul class=\"list-disc ml-6\">\n",
        )
}

/// Rails' answers, spelled out.
const MESSAGES_FOR: &str = r#"a = Article.new(title: "", body: "short")
a.valid?
raise a.title_problems.inspect unless a.title_problems == "Title can't be blank"
expected = [["Body is too short (minimum is 10 characters)"], [], [], ["can't be blank"], ["is too short (minimum is 10 characters)"]]
raise a.field_readers.inspect unless a.field_readers == expected
html = Views::Articles.form(a)
raise html unless html.include?("<p id=\"title_problems\">Title can&#39;t be blank</p>")
raise html unless html.include?("<p id=\"body_messages\">[is too short (minimum is 10 characters)]</p>")
odd = Article.new(title: "A title", body: "an odd body")
odd.valid?
raise odd.field_readers.inspect unless odd.field_readers == [[], ["is odd"], [], [], []]
raise odd.title_problems.inspect unless odd.title_problems == ""
puts "messages for passed"
"#;

/// Rails freezes what `full_messages_for` answers, empty or not, and not what `messages_for` does; a tableless
/// ActiveModel class answers from the readers it is given, as a record does from `Base`.
fn frozen_readers_app() -> emit_and_run::Overlay {
    messages_for_app()
        .edit(
            "app/models/article.rb",
            "  def field_readers\n",
            "  def frozen_readers
    [errors.full_messages_for(:title).frozen?, errors.full_messages_for(:comments).frozen?, errors.messages_for(:title).frozen?]
  end

  def field_readers\n",
        )
        .write(
            "app/models/signup.rb",
            "class Signup
  include ActiveModel::Validations

  def save
    errors.add(:email, \"is taken\")
    false
  end

  def email_readers
    [errors.full_messages_for(:email), errors.messages_for(\"email\"), errors.full_messages_for(:name)]
  end

  def frozen_readers
    [errors.full_messages_for(:email).frozen?, errors.full_messages_for(:name).frozen?, errors.messages_for(:email).frozen?]
  end
end
",
        )
}

const FROZEN_READERS: &str = r#"a = Article.new(title: "", body: "short")
a.valid?
raise a.frozen_readers.inspect unless a.frozen_readers == [true, true, false]
signup = Signup.new
signup.save
raise signup.email_readers.inspect unless signup.email_readers == [["Email is taken"], ["is taken"], []]
raise signup.frozen_readers.inspect unless signup.frozen_readers == [true, true, false]
puts "frozen readers passed"
"#;

#[test]
fn full_messages_for_is_frozen_and_messages_for_is_not() {
    frozen_readers_app().run_ruby(FROZEN_READERS).assert_passes();
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn full_messages_for_is_frozen_and_messages_for_is_not_on_spinel() {
    frozen_readers_app().run_spinel(FROZEN_READERS).assert_passes();
}

#[test]
fn full_messages_for_and_messages_for_read_one_attribute() {
    messages_for_app().run_ruby(MESSAGES_FOR).assert_passes();
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn full_messages_for_and_messages_for_read_one_attribute_on_spinel() {
    messages_for_app().run_spinel(MESSAGES_FOR).assert_passes();
}

/// A test reads the record's errors as the model does.
#[test]
fn a_test_reads_an_attribute_s_full_messages() {
    messages_for_app()
        .write(
            "test/models/article_messages_test.rb",
            r#"require "test_helper"

class ArticleMessagesTest < ActiveSupport::TestCase
  test "full messages for one attribute" do
    article = Article.new(title: "", body: "short")
    assert_not article.valid?
    assert_equal [ "Title can't be blank" ], article.errors.full_messages_for(:title)
    assert_equal [ "can't be blank" ], article.errors.messages_for("title")
    assert_empty article.errors.full_messages_for(:comments)
  end
end
"#,
        )
        .run_test("test/models/article_messages_test.rb")
        .assert_passes();
}

/// A local holding the errors is not an `errors` read, which is all the lowering grounds, so `check` refuses it.
#[test]
fn a_local_holding_the_errors_is_an_error() {
    let checked = emit_and_run::real_blog()
        .edit(
            "app/models/article.rb",
            "  validates :title, presence: true\n",
            "  validates :title, presence: true

  def title_problems
    collection = errors
    [collection.full_messages_for(:title), collection.messages_for(:title)]
  end
",
        )
        .check_errors();
    for method in ["full_messages_for", "messages_for"] {
        assert!(checked.iter().any(|e| e.contains(&format!("{method} lacks a shared runtime/lowering implementation"))), "{method}: {checked:#?}");
    }
}

/// `error_full_messages_for` is the shared runtime's, beside `error_messages_for`, so each target reads it as `errors[:field]`.
#[test]
fn every_target_reads_an_attribute_s_full_messages() {
    for target in [
        BuildTarget::Crystal,
        BuildTarget::Rust,
        BuildTarget::Elixir,
        BuildTarget::Typescript,
        BuildTarget::Go,
        BuildTarget::Python,
        BuildTarget::Kotlin,
        BuildTarget::Swift,
        BuildTarget::CSharp,
    ] {
        // Python refuses a tableless class's synthesized readers already, `errors[:field]`'s among them.
        let app = if matches!(target, BuildTarget::Python) {
            messages_for_app()
        } else {
            messages_for_app().write(
                "app/models/signup.rb",
                "class Signup\n  include ActiveModel::Validations\n\n  def email_problems\n    errors.full_messages_for(:email)\n  end\nend\n",
            )
        };
        let (_emitted, errors) = app.emit(target);
        assert!(errors.is_empty(), "{target:?}: {errors:#?}");
    }
}

fn dynamic_field_app() -> emit_and_run::Overlay {
    emit_and_run::real_blog().edit(
        "app/models/article.rb",
        "  validates :title, presence: true\n",
        "  validates :title, presence: true

  def problems_on(field)
    [errors.full_messages_for(field), errors.messages_for(field)]
  end
",
    )
}

/// A field that is not a literal is not grounded, as `errors.where` is not: `check` reports it, and so does the emit.
#[test]
fn a_dynamic_field_is_an_error() {
    let checked = dynamic_field_app().check_errors();
    for method in ["full_messages_for", "messages_for"] {
        assert!(checked.iter().any(|e| e.contains(&format!("{method} lacks a shared runtime/lowering implementation"))), "{method}: {checked:#?}");
    }
    let (_emitted, errors) = dynamic_field_app().emit(BuildTarget::Ruby);
    assert_eq!(
        errors.iter().filter(|e| e.contains("with a field that is not a literal is not grounded")).count(),
        2,
        "{errors:#?}"
    );
}
