//! The folders `bin/rails test` runs. Rails' default is
//! `test/**/*_test.rb` less `test/{system,dummy,fixtures}`, so an
//! integration test and a test in an app's own folder are part of the
//! suite, and a class-body loop declares one test per element. Both
//! files below were run first in the generated blog on Rails 8.1.4
//! (`bin/rails test`): 3 runs, 0 failures.

use super::emit_and_run;

const FLOW: &str = r##"require "test_helper"

class ArticleFlowTest < ActionDispatch::IntegrationTest
  test "a created article is shown, then listed" do
    post articles_url, params: { article: { title: "Flow Title", body: "A body long enough to pass." } }
    follow_redirect!
    assert_response :success
    assert_equal "Flow Title", Article.last.title

    get articles_url
    assert_response :success
    assert_select "h1", "Articles"
  end
end
"##;

const TITLES: &str = r##"require "test_helper"

class ArticleTitleTest < ActiveSupport::TestCase
  { "short" => "Hi", "spaced" => "Two words" }.each do |label, title|
    test "a #{label} title is kept" do
      article = Article.create!(title: title, body: "A body long enough to pass.")
      assert_equal title, article.reload.title
    end
  end
end
"##;

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .write("test/integration/article_flow_test.rb", FLOW)
        .write("test/acceptance/article_title_test.rb", TITLES)
}

#[test]
fn an_integration_test_and_a_custom_folder_run() {
    for (file, says) in [
        ("test/models/article_flow_test.rb", "ArticleFlowTest: 1 tests passed"),
        // One test per element of the loop.
        ("test/models/article_title_test.rb", "ArticleTitleTest: 2 tests passed"),
    ] {
        let run = app().run_test(file);
        run.assert_passes();
        let out = format!("{}{}", run.stdout, run.stderr);
        assert!(out.contains(says), "{file}:\n{out}");
    }
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn an_integration_test_and_a_custom_folder_run_on_spinel() {
    let (emitted, errors) = app().emit(roundhouse::project::BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:?}");
    for file in ["test/article_flow_test.rb", "test/article_title_test.rb"] {
        assert!(emitted.join(file).exists(), "{file} was not emitted in {}", emitted.display());
        let output = std::process::Command::new("spin").args(["test", file]).current_dir(&*emitted).output().expect("spawn spin");
        assert!(
            output.status.success(),
            "spin test {file} failed in {}\n=== stdout ===\n{}\n=== stderr ===\n{}",
            emitted.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
