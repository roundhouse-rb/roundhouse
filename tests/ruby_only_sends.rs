//! Sends the analyzer types for every target but only the Ruby family can
//! run: `Array#fetch`, `each_slice`, `each_cons`, `lazy`, an enumerator's
//! `with_object`, `Array[...]`, `Hash[...]`, `Array.try_convert` (a nested
//! splat), `include?` on a `Class` value,
//! Kernel's `Rational(...)` (what a rational literal ingests as) and
//! Kernel's `Integer(...)` and `Float(...)`.
//!
//! The other emitters render each as the method name on its receiver,
//! which nothing there defines, so every one refuses the construct by name
//! instead of exiting 0 with a call that cannot run. The Ruby family emits
//! them and builds (the runs are in `array_fetch_runs`,
//! `array_batches_typing`, `container_index_constructor`,
//! `include_query_receivers` and `rational_literal`).
//!
//! `<=>` is refused the same way, call and definition, on the targets whose
//! emit writes the Ruby operator as it is (Crystal's own `<=>` answers an
//! Int32 where the emit declares Integer as Int64). TypeScript renders a
//! numeric comparison, so that call is not refused there; a `def <=>` is.

#[path = "support/scratch.rs"]
mod scratch;
use std::process::Command;

const MODEL: &str = r#"class Article < ApplicationRecord
  #: (Array[Integer]) -> Integer
  def self.first_plus(nums)
    nums.fetch(0).succ
  end

  #: (Array[Integer]) -> Integer
  def self.slices(nums)
    nums.each_slice(2).to_a.length
  end

  #: (Array[Integer]) -> Integer
  def self.pairs(nums)
    nums.each_cons(2).to_a.length
  end

  #: (Array[Integer]) -> untyped
  def self.first_even(nums)
    nums.lazy.filter_map { |n| n * 10 if n.even? }.first
  end

  #: (Array[Integer]) -> Array[Integer]
  def self.memo(nums)
    nums.each_cons(2).with_object([]) { |pair, seen| seen << pair.first }
  end

  #: () -> Array[Integer]
  def self.built
    Array[1, 2]
  end

  #: () -> Hash[String, Integer]
  def self.table
    Hash[[["a", 1]]]
  end

  #: () -> Rational
  def self.third
    1/3r
  end

  #: (Class) -> bool
  def self.tagged?(klass) = klass.include?(Tagged)

  #: (String) -> Integer
  def self.count(text) = Integer(text)

  #: (String) -> Float
  def self.ratio(text) = Float(text)
end
"#;

const SERVICE: &str = r#"class RowUnpack
  def self.rest
    _head, (_month, *row) = [1, [2, 3]]
    row
  end
end
"#;

const REFUSED: [&str; 12] = [
    "Array#fetch",
    "Array#each_slice",
    "Array#each_cons",
    "Array#lazy",
    "Enumerator#with_object",
    "Array[]",
    "Hash[]",
    "Array.try_convert",
    "Kernel#Rational",
    "Module#include?",
    "Kernel#Integer",
    "Kernel#Float",
];

fn build(root: &std::path::Path, target: &str) -> (bool, String) {
    let out = root.join(format!("out-{target}"));
    let output = Command::new(env!("CARGO_BIN_EXE_roundhouse"))
        .args(["--target", target])
        .arg(root)
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    (output.status.success(), String::from_utf8_lossy(&output.stderr).into_owned())
}

#[test]
fn only_the_ruby_family_emits_the_ruby_only_sends() {
    let root = scratch::dir("rh_ruby_only_sends");
    for (path, text) in [
        ("db/schema.rb", "ActiveRecord::Schema.define do\n  create_table \"articles\" do |t|\n    t.string \"title\"\n  end\nend\n"),
        ("app/models/application_record.rb", "class ApplicationRecord < ActiveRecord::Base\n  self.abstract_class = true\nend\n"),
        ("app/models/tagged.rb", "module Tagged\n  def tagged = true\nend\n"),
        ("app/models/article.rb", MODEL),
        ("app/services/row_unpack.rb", SERVICE),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    for target in ["rust", "typescript", "go", "crystal", "python", "elixir"] {
        let (ok, text) = build(&root, target);
        assert!(!ok, "{target}: {text}");
        for construct in REFUSED {
            assert!(
                text.contains(&format!("error[unsupported]: {construct} not supported ({target})")),
                "{target} must refuse {construct}: {text}"
            );
        }
    }
    for target in ["ruby", "jruby", "spinel"] {
        let (ok, text) = build(&root, target);
        assert!(ok, "{target}: {text}");
        assert!(!text.contains("only the Ruby targets implement it"), "{target}: {text}");
    }
    std::fs::remove_dir_all(&root).unwrap();
}

/// An app method named like the Kernel conversion is the app's own, so a
/// call to it is not refused as Kernel's.
#[test]
fn an_app_method_named_integer_is_not_refused_as_kernel_s() {
    let root = scratch::dir("rh_ruby_only_sends_shadow");
    for (path, text) in [
        ("db/schema.rb", "ActiveRecord::Schema.define do\n  create_table \"articles\" do |t|\n    t.string \"title\"\n  end\nend\n"),
        ("app/models/application_record.rb", "class ApplicationRecord < ActiveRecord::Base\n  self.abstract_class = true\nend\n"),
        ("app/models/article.rb", "class Article < ApplicationRecord\n  #: (String) -> Integer\n  def Integer(text) = text.length\n\n  #: () -> Integer\n  def title_length = Integer(title.to_s)\nend\n"),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let (_ok, text) = build(&root, "rust");
    assert!(!text.contains("Kernel#Integer not supported"), "{text}");
    std::fs::remove_dir_all(&root).unwrap();
}

const SPACESHIP: &str = r#"class Article < ApplicationRecord
  def <=>(other)
    id <=> other.id
  end

  #: (Integer, Integer) -> Integer
  def self.order(a, b)
    a <=> b
  end

  #: (Integer, Integer) -> Integer
  def self.reversed(a, b)
    (a <=> b) * -1
  end

  #: (Float, Float) -> Integer?
  def self.floats(a, b)
    a <=> b
  end

  #: (Symbol, Symbol) -> Integer
  def self.symbols(a, b)
    a <=> b
  end

  #: (Article, Article) -> Integer?
  def self.records(a, b)
    a <=> b
  end

  #: (Array[Integer]) -> Array[Integer]
  def self.sorted(xs)
    xs.sort { |x, y| x <=> y }
  end

  #: (Array[Integer]) -> Array[Integer]
  def self.sorted_after_a_step(xs)
    xs.sort do |x, y|
      _gap = x.to_s
      x <=> y
    end
  end

  #: (String, String) -> Integer
  def self.strings(a, b)
    a <=> b
  end
end
"#;

/// `<=>` refusals a target reports, by its own message: (calls, definitions).
fn spaceship_refusals(text: &str, target: &str) -> (usize, usize) {
    (
        text.matches(&format!("error[unsupported]: <=> not supported ({target})")).count(),
        text.matches(&format!("error[unsupported]: def <=> not supported ({target})")).count(),
    )
}

#[test]
fn the_comparison_operator_is_refused_where_the_emit_writes_it_as_it_is() {
    let root = scratch::dir("rh_ruby_only_sends_spaceship");
    for (path, text) in [
        ("db/schema.rb", "ActiveRecord::Schema.define do\n  create_table \"articles\" do |t|\n    t.string \"title\"\n  end\nend\n"),
        ("app/models/application_record.rb", "class ApplicationRecord < ActiveRecord::Base\n  self.abstract_class = true\nend\n"),
        ("app/models/article.rb", SPACESHIP),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    // Nine calls (Integer, reversed Integer, Float, Symbol, Article, the two sort blocks, String, and the one
    // inside `def <=>`) and the definition.
    for target in ["elixir", "go", "kotlin", "python", "rust", "swift", "csharp"] {
        let (ok, text) = build(&root, target);
        assert!(!ok, "{target}: {text}");
        assert_eq!(spaceship_refusals(&text, target), (9, 1), "{target}: {text}");
    }
    // Crystal's own `<=>` is right as the value of a sort block (its last statement), and nowhere else here.
    let (ok, text) = build(&root, "crystal");
    assert!(!ok, "crystal: {text}");
    assert_eq!(spaceship_refusals(&text, "crystal"), (7, 1), "crystal: {text}");
    // TypeScript renders the numbers and refuses String by its own table: here the Symbol, Article and the two
    // untyped sort-block receivers. The definition is refused too: TypeScript would drop it.
    let (ok, text) = build(&root, "typescript");
    assert!(!ok, "typescript: {text}");
    assert_eq!(spaceship_refusals(&text, "typescript"), (4, 1), "typescript: {text}");
    assert!(text.contains("error[unsupported]: String#<=> not supported (typescript)"), "typescript: {text}");
    for target in ["ruby", "jruby", "spinel"] {
        let (ok, text) = build(&root, target);
        assert!(ok, "{target}: {text}");
        assert!(!text.contains("<=> not supported"), "{target}: {text}");
    }
    std::fs::remove_dir_all(&root).unwrap();
}

/// Test files: a `def <=>` (whose body calls no `<=>`) in a class a test file defines, and one in the test class
/// itself, are refused like a model's. The files and classes before them define no `<=>`, so a scan that reads only
/// the first test file or the first class finds nothing.
const PLAIN_TEST: &str = r#"require "test_helper"

class AardvarkTest < ActiveSupport::TestCase
  class Plain
    def n = 1
  end

  test "plain" do
    assert Plain.new.n == 1
  end
end
"#;

const OWN_DEF_TEST: &str = r#"require "test_helper"

class OwnDefTest < ActiveSupport::TestCase
  def <=>(other)
    0
  end

  test "own" do
    assert true
  end
end
"#;

const SPACESHIP_TEST: &str = r#"require "test_helper"

class VersionTest < ActiveSupport::TestCase
  class Label
    def text = "v"
  end

  class Version
    include Comparable
    attr_reader :n

    def initialize(n)
      @n = n
    end

    def <=>(other)
      n - other.n
    end
  end

  test "versions order" do
    assert Version.new(1) < Version.new(2)
  end
end
"#;

#[test]
fn a_test_files_own_class_defining_the_comparison_operator_is_refused_too() {
    let root = scratch::dir("rh_ruby_only_sends_spaceship_test_class");
    for (path, text) in [
        ("db/schema.rb", "ActiveRecord::Schema.define do\n  create_table \"articles\" do |t|\n    t.string \"title\"\n  end\nend\n"),
        ("app/models/application_record.rb", "class ApplicationRecord < ActiveRecord::Base\n  self.abstract_class = true\nend\n"),
        ("app/models/article.rb", "class Article < ApplicationRecord\nend\n"),
        ("test/test_helper.rb", "ENV[\"RAILS_ENV\"] ||= \"test\"\nrequire_relative \"../config/environment\"\nrequire \"rails/test_help\"\n"),
        ("test/models/aardvark_test.rb", PLAIN_TEST),
        ("test/models/own_def_test.rb", OWN_DEF_TEST),
        ("test/models/version_test.rb", SPACESHIP_TEST),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    // Crystal, Kotlin, Rust, Swift and C# emit these classes, and TypeScript would drop their `def <=>`; Go, Elixir
    // and Python leave a test file's classes out, and are refused like the calls' walk refuses a `<=>` call there.
    for target in ["crystal", "kotlin", "rust", "swift", "csharp", "go", "elixir", "python", "typescript"] {
        let (ok, text) = build(&root, target);
        assert!(!ok, "{target}: {text}");
        assert_eq!(spaceship_refusals(&text, target), (0, 2), "{target}: {text}");
        let refused: Vec<&str> =
            text.lines().filter(|line| line.contains(&format!("def <=> not supported ({target})"))).collect();
        for file in ["own_def_test.rb:", "version_test.rb:"] {
            assert!(refused.iter().any(|line| line.contains(file)), "{target}, {file}: {text}");
        }
    }
    for target in ["ruby", "jruby", "spinel"] {
        let (ok, text) = build(&root, target);
        assert!(ok, "{target}: {text}");
        assert!(!text.contains("<=> not supported"), "{target}: {text}");
    }
    std::fs::remove_dir_all(&root).unwrap();
}

/// Outside models and test files: a controller's action, its class method, another controller's private method, a
/// third controller's `alias_method`, a `has_many` extension's method, a model's `alias`, a plain class, and the
/// application class's method each define `<=>`. None of them calls `<=>`, so only the definitions' scan can refuse them. The plain class
/// also defines `<=`, which is not `<=>`.
const SPACESHIP_ACTIONS: &str = r#"class ArticlesController < ApplicationController
  def index
    @articles = Article.all
  end

  def <=>(other)
    0
  end

  def self.<=>(other)
    0
  end
end
"#;

const SPACESHIP_PRIVATE: &str = r#"class CommentsController < ApplicationController
  def index
    @comments = Comment.all
  end

  private

  def <=>(other)
    0
  end
end
"#;

const SPACESHIP_EXTENSION: &str = r#"class Article < ApplicationRecord
  has_many :comments do
    def <=>(other)
      0
    end
  end
end
"#;

const SPACESHIP_LIBRARY: &str = r#"class Version
  def <=>(other)
    0
  end

  def <=(other)
    true
  end
end
"#;

const SPACESHIP_CONTROLLER_ALIAS: &str = r#"class VersionsController < ApplicationController
  def index
    @count = 0
  end

  alias_method :<=>, :index
end
"#;

const SPACESHIP_ALIAS: &str = r#"class Comment < ApplicationRecord
  belongs_to :article

  def cmp(other)
    0
  end
  alias <=> cmp
end
"#;

const SPACESHIP_APPLICATION: &str = r#"require_relative "boot"
require "rails/all"

module Blog
  class Application < Rails::Application
    config.load_defaults 8.0

    def <=>(other)
      0
    end
  end
end
"#;

#[test]
fn a_controller_an_extension_or_the_application_defining_the_comparison_operator_is_refused_too() {
    let root = scratch::dir("rh_ruby_only_sends_spaceship_holders");
    for (path, text) in [
        (
            "db/schema.rb",
            "ActiveRecord::Schema.define do\n  create_table \"articles\" do |t|\n    t.string \"title\"\n  end\n\n  \
             create_table \"comments\" do |t|\n    t.integer \"article_id\"\n    t.string \"body\"\n  end\nend\n",
        ),
        ("app/models/application_record.rb", "class ApplicationRecord < ActiveRecord::Base\n  self.abstract_class = true\nend\n"),
        ("app/models/article.rb", SPACESHIP_EXTENSION),
        ("app/models/comment.rb", SPACESHIP_ALIAS),
        ("app/models/version.rb", SPACESHIP_LIBRARY),
        ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
        ("app/controllers/articles_controller.rb", SPACESHIP_ACTIONS),
        ("app/controllers/comments_controller.rb", SPACESHIP_PRIVATE),
        ("app/controllers/versions_controller.rb", SPACESHIP_CONTROLLER_ALIAS),
        ("app/views/articles/index.html.erb", "<p><%= @articles.size %></p>\n"),
        ("app/views/comments/index.html.erb", "<p><%= @comments.size %></p>\n"),
        ("app/views/versions/index.html.erb", "<p><%= @count %></p>\n"),
        (
            "config/routes.rb",
            "Rails.application.routes.draw do\n  resources :articles, only: [:index]\n  resources :comments, only: [:index]\n  \
             resources :versions, only: [:index]\nend\n",
        ),
        ("config/application.rb", SPACESHIP_APPLICATION),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    for target in ["crystal", "kotlin", "rust", "swift", "csharp", "go", "elixir", "python", "typescript"] {
        let (ok, text) = build(&root, target);
        assert!(!ok, "{target}: {text}");
        assert_eq!(spaceship_refusals(&text, target), (0, 8), "{target}: {text}");
        let refused: Vec<&str> =
            text.lines().filter(|line| line.contains(&format!("def <=> not supported ({target})"))).collect();
        for (file, count) in [
            ("/articles_controller.rb:", 2),
            ("/comments_controller.rb:", 1),
            ("/versions_controller.rb:", 1),
            ("/article.rb:", 1),
            ("/comment.rb:", 1),
            ("/version.rb:", 1),
            ("/application.rb:", 1),
        ] {
            assert_eq!(refused.iter().filter(|line| line.contains(file)).count(), count, "{target}, {file}: {text}");
        }
        // At the method's name, not at its body or its class.
        assert!(refused.iter().any(|line| line.contains("/version.rb:2:7:")), "{target}: {text}");
    }
    for target in ["ruby", "jruby", "spinel"] {
        let (ok, text) = build(&root, target);
        assert!(ok, "{target}: {text}");
        assert!(!text.contains("<=> not supported"), "{target}: {text}");
    }
    std::fs::remove_dir_all(&root).unwrap();
}
