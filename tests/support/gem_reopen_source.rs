//! A plain-Ruby gem the app's code names, compiled from source on Spinel,
//! whose class the app reopens in `lib/` from an initializer, as an app
//! adds to a gem's class. Ruby loads the gem first (Bundler) and the
//! reopen after it, so the class holds the gem's methods and the app's,
//! and a method both define is the app's. Shared by the Ruby lane and the
//! Spinel lane; the answers are Ruby 4.0.7's.

use super::emit_and_run;

pub const GEM_DIR: &str = "vendor/bundle/ruby/3.4.0/gems/shelf-1.0.0";

pub const SHELF: &str = r##"module Shelf
  class Box
    def self.kind
      "box"
    end

    def self.label
      "shelf box"
    end
  end
end
"##;

/// Adds a method and redefines one the gem has.
pub const LOCAL: &str = r##"module Shelf
  class Box
    def self.local_tag
      "local"
    end

    def self.label
      "app box"
    end
  end
end
"##;

/// [`LOCAL`] in the compact form, as an initializer often writes it.
pub const LOCAL_COMPACT: &str = r##"class Shelf::Box
  def self.local_tag
    "local"
  end

  def self.label
    "app box"
  end
end
"##;

/// [`LOCAL`] as a `class_eval` block in an initializer: `self` in the
/// block is the class, so `def self.…` defines its class methods.
pub const CLASS_EVAL: &str = r##"Shelf::Box.class_eval do
  def self.local_tag
    "local"
  end

  def self.label
    "app box"
  end
end
"##;

/// A `class_eval` block that also writes a constant. Ruby puts `LIMIT` in
/// the block's lexical scope (the top level), not in `Shelf::Box`, so the
/// block is not the same as a reopen and is left as it is.
pub const CLASS_EVAL_LIMIT: &str = r##"Shelf::Box.class_eval do
  LIMIT = 3

  def self.local_tag
    "local"
  end
end
"##;

/// Adds only a constant and an `include`, no method.
pub const EXTRAS: &str = r##"module Shelf
  class Box
    LIMIT = 3
    include Tagging
  end
end
"##;

pub const TAGGING: &str = r##"module Tagging
  def tag
    "tagged"
  end
end
"##;

pub const ARTICLE_BOX: &str = r##"class ArticleBox
  def self.describe
    [Shelf::Box.kind, Shelf::Box.label, Shelf::Box.local_tag].join(" / ")
  end
end
"##;

/// The app's use of the class a constant-only reopen extends. It names
/// `Shelf::Box` alone: a constant path under it (`Shelf::Box::LIMIT`) is
/// not a name the app declares, and brought the gem in already.
pub const BOX_LIMITS: &str = r##"class BoxLimits
  def self.describe
    [Shelf::Box.kind, Shelf::Box.new.tag]
  end
end
"##;

pub const LOCAL_ASSERTIONS: &str = r##"got = ArticleBox.describe
raise "describe #{got.inspect}" unless got == "box / app box / local"
puts "reopen passed"
"##;

/// [`models_app`]: Rails 8.1.4.1 never loads the reopen (Zeitwerk loads
/// `app/models/shelf/box.rb` only for an undefined `Shelf::Box`, and the gem
/// defined it), with eager loading or without, so the class is the gem's.
pub const MODELS_ASSERTIONS: &str = r##"got = [Shelf::Box.kind, Shelf::Box.label, Shelf::Box.respond_to?(:local_tag)]
raise "models #{got.inspect}" unless got == ["box", "shelf box", false]
puts "models passed"
"##;

/// [`required_models_app`]: Rails 8.1.4.1 loads a file an initializer
/// requires by path, autoload path or not, so the reopen applies.
pub const REQUIRED_MODELS_ASSERTIONS: &str = r##"got = [Shelf::Box.kind, Shelf::Box.label, Shelf::Box.respond_to?(:local_tag)]
raise "required models #{got.inspect}" unless got == ["box", "app box", true]
puts "required models passed"
"##;

pub const EXTRAS_ASSERTIONS: &str = r##"got = BoxLimits.describe + [Shelf::Box::LIMIT]
raise "extras #{got.inspect}" unless got == ["box", "tagged", 3]
puts "extras passed"
"##;

/// real-blog with the `shelf` gem installed under `vendor/bundle` and
/// locked, but not listed in `source_gems`: the app's naming of
/// `Shelf::Box` is what brings it in.
fn with_gem() -> emit_and_run::Overlay {
    with_gem_lib(SHELF)
}

/// [`with_gem`] with `lib/shelf.rb` holding `lib`.
fn with_gem_lib(lib: &str) -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .edit("Gemfile.lock", "    debug (1.11.1)\n", "    debug (1.11.1)\n    shelf (1.0.0)\n")
        .edit("Gemfile.lock", "DEPENDENCIES\n", "DEPENDENCIES\n  shelf\n")
        .edit("Gemfile", "gem \"rails\", \"~> 8.1.4\"\n", "gem \"rails\", \"~> 8.1.4\"\ngem \"shelf\"\n")
        .write(".bundle/config", "---\nBUNDLE_PATH: \"vendor/bundle\"\n")
        .write(&format!("{GEM_DIR}/lib/shelf.rb"), lib)
}

/// The same app with the gem listed in `source_gems`, which compiles it
/// from source on every target: the Ruby family otherwise loads the
/// installed gem through its Gemfile, which the test tree does not have.
#[allow(dead_code)]
pub fn listed(app: emit_and_run::Overlay) -> emit_and_run::Overlay {
    app.write("roundhouse.yml", "extensions:\n  - config/roundhouse\n")
        .write("config/roundhouse/gems.yml", "source_gems:\n  - {gem: \"shelf\"}\n")
}

/// The reopen that adds `local_tag` and redefines `label`.
#[allow(dead_code)]
pub fn local_app() -> emit_and_run::Overlay {
    with_gem()
        .write("lib/shelf_box_local.rb", LOCAL)
        .write("config/initializers/shelf_box_local.rb", "require Rails.root.join(\"lib/shelf_box_local\").to_s\n")
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// [`LOCAL`] at `app/models/shelf/box.rb`, the path Zeitwerk maps to
/// `Shelf::Box`, with no initializer requiring it (#2093's third form).
#[allow(dead_code)]
pub fn models_app() -> emit_and_run::Overlay {
    with_gem().write("app/models/shelf/box.rb", LOCAL)
}

/// [`models_app`] with an initializer requiring the file by path.
#[allow(dead_code)]
pub fn required_models_app() -> emit_and_run::Overlay {
    models_app().write("config/initializers/shelf_box.rb", "require Rails.root.join(\"app/models/shelf/box\").to_s\n")
}

/// [`models_app`] with the require split into `Rails.root.join` arguments.
#[allow(dead_code)]
pub fn split_required_models_app() -> emit_and_run::Overlay {
    models_app().write("config/initializers/shelf_box.rb", "require Rails.root.join(\"app\", \"models\", \"shelf\", \"box\").to_s\n")
}

/// [`models_app`] with an initializer loading `lib/patches.rb`, which loads the reopen: both by `require_relative`.
#[allow(dead_code)]
pub fn chain_required_models_app() -> emit_and_run::Overlay {
    models_app()
        .write("config/initializers/patches.rb", "require_relative \"../../lib/patches\"\n")
        .write("lib/patches.rb", "require_relative \"../app/models/shelf/box\"\n")
}

/// [`models_app`] with the require only in a comment.
#[allow(dead_code)]
pub fn commented_required_models_app() -> emit_and_run::Overlay {
    models_app().write("config/initializers/shelf_box.rb", "# require Rails.root.join(\"app/models/shelf/box\").to_s\n")
}

/// The gem's `Shelf::Box` assigned (`Box = Class.new(Base)`) rather than declared.
pub const SHELF_ASSIGNED: &str = r##"module Shelf
  class Base
    def self.kind
      "box"
    end

    def self.label
      "shelf box"
    end
  end

  Box = Class.new(Base)
end
"##;

/// The reopen at `app/models/shelf/box.rb` of a gem class it assigns.
#[allow(dead_code)]
pub fn assigned_models_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_ASSIGNED).write("app/models/shelf/box.rb", LOCAL)
}

/// The gem's class under a namespace the app's acronym inflection names (`API`).
pub const SHELF_API: &str = r##"module Shelf
end

module API
  class Box
    def self.kind
      "box"
    end

    def self.label
      "shelf box"
    end
  end
end
"##;

/// The reopen at `app/models/api/box.rb`, which Zeitwerk maps to `API::Box` under `inflect.acronym "API"`.
#[allow(dead_code)]
pub fn acronym_models_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_API)
        .write("config/initializers/inflections.rb", "ActiveSupport::Inflector.inflections(:en) do |inflect|\n  inflect.acronym \"API\"\nend\n")
        .write("app/models/api/box.rb", &LOCAL.replace("module Shelf", "module API"))
}

/// A gem whose `Shelf::Box` is in a file of its own, `lib/shelf/box.rb`,
/// which `lib/shelf.rb` doesn't require.
fn with_unrequired_gem_box() -> emit_and_run::Overlay {
    with_gem_lib("module Shelf\nend\n").write(&format!("{GEM_DIR}/lib/shelf/box.rb"), SHELF)
}

/// [`models_app`] with the gem's `Shelf::Box` in `lib/shelf/box.rb`, which
/// nothing requires.
#[allow(dead_code)]
pub fn unrequired_gem_box_models_app() -> emit_and_run::Overlay {
    with_unrequired_gem_box().write("app/models/shelf/box.rb", LOCAL)
}

/// [`models_app`] with the gem's `Shelf::Box` in `lib/shelf/box.rb`, which
/// the gem autoloads by feature name (`autoload :Box, "shelf/box"`).
#[allow(dead_code)]
pub fn feature_autoloaded_gem_box_models_app() -> emit_and_run::Overlay {
    with_gem_lib("module Shelf\n  autoload :Box, \"shelf/box\"\nend\n")
        .write(&format!("{GEM_DIR}/lib/shelf/box.rb"), SHELF)
        .write("app/models/shelf/box.rb", LOCAL)
}

/// [`unrequired_gem_box_models_app`]: the gem has not defined `Shelf::Box`
/// when Zeitwerk scans `app/models`, so Rails 8.1.4.1 loads the app's file,
/// with eager loading or without, and never the gem's.
#[allow(dead_code)]
pub const APP_FILE_MODELS_ASSERTIONS: &str = r##"got = [Shelf::Box.respond_to?(:kind), Shelf::Box.label, Shelf::Box.respond_to?(:local_tag)]
raise "app file models #{got.inspect}" unless got == [false, "app box", true]
puts "app file models passed"
"##;

/// [`feature_autoloaded_gem_box_models_app`]: an autoload to a feature name
/// doesn't count (Zeitwerk 2.8.3 skips a file only for a defined constant
/// or an autoload to a `.rb` path), so Rails 8.1.4.1 loads the app's file,
/// with eager loading or without. It never reads the gem's either, so
/// `kind` is undefined there, but roundhouse compiles every file the gem's
/// requires and autoloads reach, as main does, and that half isn't asserted.
#[allow(dead_code)]
pub const AUTOLOADED_MODELS_ASSERTIONS: &str = r##"got = [Shelf::Box.label, Shelf::Box.respond_to?(:local_tag)]
raise "autoloaded models #{got.inspect}" unless got == ["app box", true]
puts "autoloaded models passed"
"##;

/// [`acronym_models_app`]: Rails 8.1.4.1 never loads the reopen, so the class is the gem's.
pub const ACRONYM_MODELS_ASSERTIONS: &str = r##"got = [API::Box.kind, API::Box.label, API::Box.respond_to?(:local_tag)]
raise "acronym models #{got.inspect}" unless got == ["box", "shelf box", false]
puts "acronym models passed"
"##;

/// The reopen that adds only `LIMIT` and `include Tagging`.
#[allow(dead_code)]
pub fn extras_app() -> emit_and_run::Overlay {
    with_gem()
        .write("app/models/tagging.rb", TAGGING)
        .write("lib/shelf_box_extras.rb", EXTRAS)
        .write("config/initializers/shelf_box_extras.rb", "require Rails.root.join(\"lib/shelf_box_extras\").to_s\n")
        .write("app/models/box_limits.rb", BOX_LIMITS)
}

/// The reopen written in the initializer itself rather than in `lib/`:
/// Rails runs it at boot, after Bundler has loaded the gem.
#[allow(dead_code)]
pub fn initializer_app() -> emit_and_run::Overlay {
    with_gem()
        .write("config/initializers/shelf_box.rb", LOCAL)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// [`initializer_app`] with the reopen in the compact `class Shelf::Box` form.
#[allow(dead_code)]
pub fn compact_initializer_app() -> emit_and_run::Overlay {
    with_gem()
        .write("config/initializers/shelf_box.rb", LOCAL_COMPACT)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// The reopen as a `class_eval` block in an initializer (#2093's second form).
#[allow(dead_code)]
pub fn class_eval_app() -> emit_and_run::Overlay {
    with_gem()
        .write("config/initializers/shelf_box.rb", CLASS_EVAL)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// [`class_eval_app`] with a block that also writes a constant.
#[allow(dead_code)]
pub fn class_eval_limit_app() -> emit_and_run::Overlay {
    with_gem()
        .write("config/initializers/shelf_box.rb", CLASS_EVAL_LIMIT)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// An initializer's reopen with its own `label`, then the [`CLASS_EVAL`]
/// block: Ruby runs the file top to bottom, so the block's `label` stands.
#[allow(dead_code)]
pub fn reopen_then_class_eval_app() -> emit_and_run::Overlay {
    let reopen = "class Shelf::Box\n  def self.label\n    \"first\"\n  end\nend\n\n";
    with_gem()
        .write("config/initializers/shelf_box.rb", &format!("{reopen}{CLASS_EVAL}"))
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// The same across two initializers: Rails runs them in sorted order, so
/// `b_box.rb`'s block runs after `a_box.rb`'s reopen.
#[allow(dead_code)]
pub fn reopen_then_class_eval_across_files_app() -> emit_and_run::Overlay {
    let reopen = "class Shelf::Box\n  def self.label\n    \"first\"\n  end\n\n  def self.local_tag\n    \"a\"\n  end\nend\n";
    with_gem()
        .write("config/initializers/a_box.rb", reopen)
        .write("config/initializers/b_box.rb", CLASS_EVAL)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// A block whose method calls a helper module the same initializer defines
/// and nothing in `app/` names. Ruby answers `box / app box / helped`.
#[allow(dead_code)]
pub fn class_eval_helper_app() -> emit_and_run::Overlay {
    let source = "module LabelHelper\n  def self.tag\n    \"helped\"\n  end\nend\n\nShelf::Box.class_eval do\n  def self.local_tag\n    LabelHelper.tag\n  end\n\n  def self.label\n    \"app box\"\n  end\nend\n";
    with_gem()
        .write("config/initializers/shelf_box.rb", source)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// A helper the kept block calls that calls a second helper, both only in
/// the initializer. Ruby answers `box / app box / sourced`.
#[allow(dead_code)]
pub fn class_eval_transitive_helper_app() -> emit_and_run::Overlay {
    let source = "module TagSource\n  def self.tag\n    \"sourced\"\n  end\nend\n\nmodule LabelHelper\n  def self.tag\n    TagSource.tag\n  end\nend\n\nShelf::Box.class_eval do\n  def self.local_tag\n    LabelHelper.tag\n  end\n\n  def self.label\n    \"app box\"\n  end\nend\n";
    with_gem()
        .write("config/initializers/shelf_box.rb", source)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

pub const TRANSITIVE_ASSERTIONS: &str = r##"got = ArticleBox.describe
raise "describe #{got.inspect}" unless got == "box / app box / sourced"
puts "transitive passed"
"##;

/// A `class Shelf::Box` reopen in the initializer whose method calls a
/// helper only the initializer defines. Ruby answers `box / app box /
/// sourced`.
#[allow(dead_code)]
pub fn reopen_helper_app() -> emit_and_run::Overlay {
    let source = "module TagSource\n  def self.tag\n    \"sourced\"\n  end\nend\n\nclass Shelf::Box\n  def self.local_tag\n    TagSource.tag\n  end\n\n  def self.label\n    \"app box\"\n  end\nend\n";
    with_gem()
        .write("config/initializers/shelf_box.rb", source)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// A kept helper whose parameter default reads another initializer
/// helper (`def self.tag(value = TagSource.tag)`). Ruby answers the same.
#[allow(dead_code)]
pub fn class_eval_default_helper_app() -> emit_and_run::Overlay {
    let source = "module TagSource\n  def self.tag\n    \"sourced\"\n  end\nend\n\nmodule LabelHelper\n  def self.tag(value = TagSource.tag)\n    value\n  end\nend\n\nShelf::Box.class_eval do\n  def self.local_tag\n    LabelHelper.tag\n  end\n\n  def self.label\n    \"app box\"\n  end\nend\n";
    with_gem()
        .write("config/initializers/shelf_box.rb", source)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// A kept helper whose constant's value reads another initializer helper
/// (`TAG = TagSource.tag`). Ruby answers `box / app box / sourced`.
#[allow(dead_code)]
pub fn class_eval_constant_helper_app() -> emit_and_run::Overlay {
    let source = "module TagSource\n  def self.tag\n    \"sourced\"\n  end\nend\n\nmodule LabelHelper\n  TAG = TagSource.tag\n\n  def self.tag\n    TAG\n  end\nend\n\nShelf::Box.class_eval do\n  def self.local_tag\n    LabelHelper.tag\n  end\n\n  def self.label\n    \"app box\"\n  end\nend\n";
    with_gem()
        .write("config/initializers/shelf_box.rb", source)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// A kept helper that includes a second helper module only the
/// initializer defines, which nothing names in a method body
/// (`class LabelHelper; include TagSource`). Ruby answers `box / app box
/// / sourced`.
#[allow(dead_code)]
pub fn class_eval_included_helper_app() -> emit_and_run::Overlay {
    let source = "module TagSource\n  def tag\n    \"sourced\"\n  end\nend\n\nclass LabelHelper\n  include TagSource\nend\n\nShelf::Box.class_eval do\n  def self.local_tag\n    LabelHelper.new.tag\n  end\n\n  def self.label\n    \"app box\"\n  end\nend\n";
    with_gem()
        .write("config/initializers/shelf_box.rb", source)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// A kept helper that extends a second helper module only the initializer
/// defines, which nothing names in a method body
/// (`class LabelHelper; extend TagSource`). Ruby answers `box / app box
/// / sourced`.
#[allow(dead_code)]
pub fn class_eval_extended_helper_app() -> emit_and_run::Overlay {
    let source = "module TagSource\n  def tag\n    \"sourced\"\n  end\nend\n\nclass LabelHelper\n  extend TagSource\nend\n\nShelf::Box.class_eval do\n  def self.local_tag\n    LabelHelper.tag\n  end\n\n  def self.label\n    \"app box\"\n  end\nend\n";
    with_gem()
        .write("config/initializers/shelf_box.rb", source)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// A kept helper whose superclass is a second helper only the initializer
/// defines (`class LabelHelper < TagSource`). Ruby answers `box / app box
/// / sourced`.
#[allow(dead_code)]
pub fn class_eval_superclass_helper_app() -> emit_and_run::Overlay {
    let source = "class TagSource\n  def tag\n    \"sourced\"\n  end\nend\n\nclass LabelHelper < TagSource\nend\n\nShelf::Box.class_eval do\n  def self.local_tag\n    LabelHelper.new.tag\n  end\n\n  def self.label\n    \"app box\"\n  end\nend\n";
    with_gem()
        .write("config/initializers/shelf_box.rb", source)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// [`class_eval_helper_app`] with the helper named from the top level,
/// `::LabelHelper.tag`. Ruby answers the same.
#[allow(dead_code)]
pub fn class_eval_rooted_helper_app() -> emit_and_run::Overlay {
    let source = "module LabelHelper\n  def self.tag\n    \"helped\"\n  end\nend\n\nShelf::Box.class_eval do\n  def self.local_tag\n    ::LabelHelper.tag\n  end\n\n  def self.label\n    \"app box\"\n  end\nend\n";
    with_gem()
        .write("config/initializers/shelf_box.rb", source)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// A block with a `def` on another receiver (`def Other.label`). Ruby
/// defines that one on `Other`, not on `Shelf::Box`.
#[allow(dead_code)]
pub fn class_eval_other_receiver_app() -> emit_and_run::Overlay {
    let source = "module Other\nend\n\nShelf::Box.class_eval do\n  def self.local_tag\n    \"local\"\n  end\n\n  def Other.label\n    \"other\"\n  end\nend\n";
    with_gem()
        .write("config/initializers/shelf_box.rb", source)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

pub const HELPER_ASSERTIONS: &str = r##"got = ArticleBox.describe
raise "describe #{got.inspect}" unless got == "box / app box / helped"
puts "helper passed"
"##;

/// The gem's class sets a class variable, and a block's method asks
/// `defined?(@@configured)`. The block's lexical scope is the top level,
/// so Ruby 4.0.7 answers `"plain"` there; a `class Shelf::Box` reopening
/// would answer `"configured"`.
#[allow(dead_code)]
pub fn class_eval_class_variable_app() -> emit_and_run::Overlay {
    let lib = SHELF.replace("  class Box\n", "  class Box\n    @@configured = true\n\n");
    with_gem_lib(&lib)
        .write(
            "config/initializers/shelf_box.rb",
            "Shelf::Box.class_eval do\n  def self.local_tag\n    defined?(@@configured) ? \"configured\" : \"plain\"\n  end\nend\n",
        )
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// An initializer reopen gives the gem's class a `TokenSource` of its
/// own, and a later block in the same file reads `TokenSource` bare.
/// Ruby 4.0.7 reads the top level's in the block (`"top"`); an opening
/// would read the reopen's (`"shadow"`).
#[allow(dead_code)]
pub fn class_eval_reopen_constant_app() -> emit_and_run::Overlay {
    with_gem()
        .write(
            "config/initializers/shelf_box.rb",
            "TokenSource = \"top\"\n\nclass Shelf::Box\n  TokenSource = \"shadow\"\nend\n\nShelf::Box.class_eval do\n  def self.local_tag\n    TokenSource\n  end\n\n  def self.label\n    \"app box\"\n  end\nend\n",
        )
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// The gem's class with a constant of its own, and a block whose method
/// reads that name bare. The block's lexical scope is the top level, so
/// Ruby looks `TOKEN` up there: `uninitialized constant TOKEN`, not the
/// gem's `Shelf::Box::TOKEN`.
#[allow(dead_code)]
pub fn class_eval_bare_constant_app() -> emit_and_run::Overlay {
    let lib = SHELF.replace("  class Box\n", "  class Box\n    TOKEN = \"gem\"\n\n");
    with_gem_lib(&lib)
        .write("config/initializers/shelf_box.rb", "Shelf::Box.class_eval do\n  def self.local_tag\n    TOKEN\n  end\nend\n")
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// The gem's `Shelf::Box` inherits a `TokenSource` (from its superclass
/// `Shelf::Base`, or from a module it includes), and the initializer
/// defines a top-level `TokenSource` its block reads bare. Ruby 4.0.7
/// reads the top level's in the block (`"top"`), and the ancestor's in a
/// `class Shelf::Box` reopen (`"base"`).
#[allow(dead_code)]
pub fn class_eval_inherited_constant_apps() -> Vec<emit_and_run::Overlay> {
    let token = "module TokenSource\n      def self.tag\n        \"base\"\n      end\n    end\n";
    let superclass = SHELF.replace("  class Box\n", &format!("  class Base\n    {token}  end\n\n  class Box < Base\n"));
    let included = SHELF.replace("  class Box\n", &format!("  module Tokens\n    {token}  end\n\n  class Box\n    include Tokens\n\n"));
    let initializer = "module TokenSource\n  def self.tag\n    \"top\"\n  end\nend\n\nShelf::Box.class_eval do\n  def self.local_tag\n    TokenSource.tag\n  end\nend\n";
    [superclass, included]
        .iter()
        .map(|lib| with_gem_lib(lib).write("config/initializers/shelf_box.rb", initializer).write("app/models/article_box.rb", ARTICLE_BOX))
        .collect()
}

/// A kept helper whose class body sets a class ivar from a second helper
/// only the initializer defines (`@tag = TagSource.tag`). Ruby 4.0.7
/// answers `box / app box / sourced`.
#[allow(dead_code)]
pub fn class_eval_class_ivar_helper_app() -> emit_and_run::Overlay {
    let source = "module TagSource\n  def self.tag\n    \"sourced\"\n  end\nend\n\nclass LabelHelper\n  @tag = TagSource.tag\n\n  def self.tag\n    @tag\n  end\nend\n\nShelf::Box.class_eval do\n  def self.local_tag\n    LabelHelper.tag\n  end\n\n  def self.label\n    \"app box\"\n  end\nend\n";
    with_gem()
        .write("config/initializers/shelf_box.rb", source)
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// A block the ingest refuses (`defined?` of a call with arguments), then
/// [`CLASS_EVAL`]. Ruby runs both; the second is a reopen like any other.
#[allow(dead_code)]
pub fn refused_then_class_eval_app() -> emit_and_run::Overlay {
    let refused = "Shelf::Box.class_eval do\n  def self.probe\n    defined?(String.new(1)) ? 1 : 0\n  end\nend\n\n";
    with_gem()
        .write("config/initializers/shelf_box.rb", &format!("{refused}{CLASS_EVAL}"))
        .write("app/models/article_box.rb", ARTICLE_BOX)
}

/// A gem module a controller includes.
pub const SHELF_TAGGING: &str = r##"module Shelf
  module Tagging
    def original
      "gem"
    end

    def label
      "gem label"
    end
  end
end
"##;

/// An initializer's reopen of it: a method of its own, and `label` redefined.
pub const TAGGING_REOPEN: &str = r##"module Shelf
  module Tagging
    def local
      "local"
    end

    def label
      "app label"
    end
  end
end
"##;

pub const SHELVES_CONTROLLER: &str = r##"class ShelvesController < ApplicationController
  include Shelf::Tagging

  def show
    render plain: [original, local, label].join(" / ")
  end
end
"##;

/// The controller including the gem module the initializer reopens.
/// Ruby answers `"gem / local / app label"` for `show`.
#[allow(dead_code)]
pub fn included_module_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_TAGGING)
        .write("config/initializers/shelf_tagging.rb", TAGGING_REOPEN)
        .write("app/controllers/shelves_controller.rb", SHELVES_CONTROLLER)
}

/// [`included_module_app`] with the reopen in `lib/`, required by an
/// initializer (#2122's form). Ruby answers the same.
#[allow(dead_code)]
pub fn included_module_lib_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_TAGGING)
        .write("lib/shelf_tagging.rb", TAGGING_REOPEN)
        .write("config/initializers/shelf_tagging.rb", "require Rails.root.join(\"lib/shelf_tagging\").to_s\n")
        .write("app/controllers/shelves_controller.rb", SHELVES_CONTROLLER)
}

/// A gem module with an empty body.
pub const SHELF_HOOKS: &str = r##"module Shelf
  module Hooks
  end
end
"##;

pub const HOOKS_REOPEN: &str = r##"module Shelf
  module Hooks
    def self.hook
      "hooked"
    end
  end
end
"##;

pub const HOOK_PROBE: &str = r##"class HookProbe
  def self.describe
    Shelf::Hooks.hook
  end
end
"##;

/// An initializer adding a method to a module the gem declares empty.
/// Ruby answers `"hooked"` for `HookProbe.describe`.
#[allow(dead_code)]
pub fn empty_module_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_HOOKS)
        .write("config/initializers/shelf_hooks.rb", HOOKS_REOPEN)
        .write("app/models/hook_probe.rb", HOOK_PROBE)
}

/// A gem module whose only member is an `attr_reader`.
pub const SHELF_ATTR: &str = r##"module Shelf
  module Box
    attr_reader :value
  end
end
"##;

/// An initializer's reopen of it, adding a module method.
pub const ATTR_REOPEN: &str = r##"module Shelf
  module Box
    def self.local_tag
      "local"
    end
  end
end
"##;

pub const ATTR_PROBE: &str = r##"class AttrProbe
  def self.describe
    Shelf::Box.local_tag
  end
end
"##;

/// An initializer adding a method to a gem module that has only an
/// `attr_reader`. Ruby answers `"local"` for `AttrProbe.describe`.
#[allow(dead_code)]
pub fn attr_module_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_ATTR)
        .write("config/initializers/shelf_box.rb", ATTR_REOPEN)
        .write("app/models/attr_probe.rb", ATTR_PROBE)
}

/// A reopen of [`SHELF_TAGGING`] that defines `label` only under a
/// load-time `if` whose condition is false.
pub const TAGGING_CONDITIONAL: &str = r##"module Shelf
  module Tagging
    FLAG = false

    def local
      "local"
    end

    if FLAG
      def label
        "app label"
      end
    end
  end
end
"##;

/// The controller including the gem module, the initializer's `label`
/// under a false condition. Ruby answers `"gem / local / gem label"`.
#[allow(dead_code)]
pub fn conditional_reopen_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_TAGGING)
        .write("config/initializers/shelf_tagging.rb", TAGGING_CONDITIONAL)
        .write("app/controllers/shelves_controller.rb", SHELVES_CONTROLLER)
}

/// [`conditional_reopen_app`] with the constant true. Ruby answers
/// `"gem / local / app label"`; a constant's value is not known where the
/// reopen is spliced, so the emit keeps the gem's `label`.
#[allow(dead_code)]
pub fn true_conditional_reopen_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_TAGGING)
        .write("config/initializers/shelf_tagging.rb", &TAGGING_CONDITIONAL.replace("FLAG = false", "FLAG = true"))
        .write("app/controllers/shelves_controller.rb", SHELVES_CONTROLLER)
}

/// A reopen of [`SHELF_TAGGING`] that defines `label` twice.
pub const TAGGING_TWICE: &str = r##"module Shelf
  module Tagging
    def local
      "local"
    end

    def label
      "first label"
    end

    def label
      "app label"
    end
  end
end
"##;

/// The controller including the gem module, the initializer defining
/// `label` twice. Ruby answers `"gem / local / app label"`: the later
/// definition stands.
#[allow(dead_code)]
pub fn twice_reopen_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_TAGGING)
        .write("config/initializers/shelf_tagging.rb", TAGGING_TWICE)
        .write("app/controllers/shelves_controller.rb", SHELVES_CONTROLLER)
}

/// [`included_module_app`] with the reopen in `app/models/shelf_tagging_ext.rb`,
/// a file that also defines its own `ShelfTaggingExt`. The models are read
/// before the gem's files, but Ruby loads the gem first (Bundler, before
/// Rails eager-loads `app/models`) and answers `"gem / local / app label"`.
#[allow(dead_code)]
pub fn models_reopen_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_TAGGING)
        .write("app/models/shelf_tagging_ext.rb", &format!("module ShelfTaggingExt\nend\n\n{TAGGING_REOPEN}"))
        .write("app/controllers/shelves_controller.rb", SHELVES_CONTROLLER)
}

/// A reopen of [`SHELF_TAGGING`] whose `label` is defined under `condition`.
#[allow(dead_code)]
fn tagging_under(condition: &str) -> String {
    TAGGING_CONDITIONAL.replace("    FLAG = false\n\n", "").replace("if FLAG", condition)
}

/// The initializer defines `label` under `if defined?(Shelf::Tagging)`, a
/// constant the program carries. Ruby answers `"gem / local / app label"`.
#[allow(dead_code)]
pub fn defined_reopen_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_TAGGING)
        .write("config/initializers/shelf_tagging.rb", &tagging_under("if defined?(Shelf::Tagging)"))
        .write("app/controllers/shelves_controller.rb", SHELVES_CONTROLLER)
}

/// [`defined_reopen_app`] with `unless`. Ruby answers
/// `"gem / local / gem label"`.
#[allow(dead_code)]
pub fn undefined_reopen_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_TAGGING)
        .write("config/initializers/shelf_tagging.rb", &tagging_under("unless defined?(Shelf::Tagging)"))
        .write("app/controllers/shelves_controller.rb", SHELVES_CONTROLLER)
}

/// The initializer defines `label` under `if ENV["SHELF_LABEL"]`, which only
/// running the program answers. Ruby answers `"gem / local / gem label"`
/// with the variable unset and `"gem / local / app label"` with it set.
#[allow(dead_code)]
pub fn env_reopen_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_TAGGING)
        .write("config/initializers/shelf_tagging.rb", &tagging_under("if ENV[\"SHELF_LABEL\"]"))
        .write("app/controllers/shelves_controller.rb", SHELVES_CONTROLLER)
}

/// A reopen of [`SHELF_TAGGING`] defining `self.enabled` under `{enabled}`,
/// then `label` under `if respond_to?(:enabled)`.
const TAGGING_RESPOND: &str = r##"module Shelf
  module Tagging
    {enabled}
      def self.enabled
        true
      end
    end

    def local
      "local"
    end

    if respond_to?(:enabled)
      def label
        "app label"
      end
    end
  end
end
"##;

fn respond_reopen_app(enabled: &str) -> emit_and_run::Overlay {
    with_gem_lib(SHELF_TAGGING)
        .write("config/initializers/shelf_tagging.rb", &TAGGING_RESPOND.replace("{enabled}", enabled))
        .write("app/controllers/shelves_controller.rb", SHELVES_CONTROLLER)
}

/// `self.enabled` under `unless defined?(Shelf::Tagging)`, which is never
/// defined, so `respond_to?(:enabled)` is false. Ruby answers
/// `"gem / local / gem label"`.
#[allow(dead_code)]
pub fn respond_after_skipped_app() -> emit_and_run::Overlay {
    respond_reopen_app("unless defined?(Shelf::Tagging)")
}

/// `self.enabled` under `if ENV["SHELF_ENABLED"]`, which only running the
/// program answers. Ruby answers `"gem / local / gem label"` with the
/// variable unset and `"gem / local / app label"` with it set.
#[allow(dead_code)]
pub fn respond_after_unknown_app() -> emit_and_run::Overlay {
    respond_reopen_app("if ENV[\"SHELF_ENABLED\"]")
}

/// `self.enabled` under `if defined?(Shelf::Tagging)`, so
/// `respond_to?(:enabled)` is true. Ruby answers `"gem / local / app label"`.
#[allow(dead_code)]
pub fn respond_after_settled_app() -> emit_and_run::Overlay {
    respond_reopen_app("if defined?(Shelf::Tagging)")
}

/// `label` under `if respond_to?(:enabled)`, written before `self.enabled`
/// under `if defined?(Shelf::Tagging)`: the `respond_to?` runs before the
/// method is defined. Ruby answers `"gem / local / gem label"`.
#[allow(dead_code)]
pub fn respond_before_settled_app() -> emit_and_run::Overlay {
    let reopen = "module Shelf\n  module Tagging\n    def local\n      \"local\"\n    end\n\n    if respond_to?(:enabled)\n      def label\n        \"app label\"\n      end\n    end\n\n    if defined?(Shelf::Tagging)\n      def self.enabled\n        true\n      end\n    end\n  end\nend\n";
    with_gem_lib(SHELF_TAGGING)
        .write("config/initializers/shelf_tagging.rb", reopen)
        .write("app/controllers/shelves_controller.rb", SHELVES_CONTROLLER)
}

/// A gem class whose body only sets a class instance variable.
pub const SHELF_IVAR: &str = r##"module Shelf
  class Box
    @configured = true
  end
end
"##;

/// An initializer adding a class method to a gem class whose only member
/// is a class instance variable. Ruby answers `"local"` for
/// `AttrProbe.describe`.
#[allow(dead_code)]
pub fn ivar_class_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_IVAR)
        .write("config/initializers/shelf_box.rb", &ATTR_REOPEN.replace("module Box", "class Box"))
        .write("app/models/attr_probe.rb", ATTR_PROBE)
}

/// A gem `Shelf::Base` with a method of its own, so its opening is the
/// gem's, and `Shelf::Child < Shelf::Base` (the superclass written in full) whose `label` answers `"gem label"`.
pub const SHELF_KINDS: &str = r##"module Shelf
  class Base
    def self.kind
      "base"
    end
  end

  class Child < Shelf::Base
    def label
      "gem label"
    end
  end
end
"##;

/// An initializer reopening `Child` with `label` under
/// `if respond_to?(:enabled)`, then `Base` with `self.enabled` under
/// `if defined?(Shelf::Base)`: the `respond_to?` runs before `enabled` is
/// defined.
pub const KINDS_REOPEN: &str = r##"module Shelf
  class Child
    if respond_to?(:enabled)
      def label
        "app label"
      end
    end
  end

  class Base
    if defined?(Shelf::Base)
      def self.enabled
        true
      end
    end
  end
end
"##;

/// Ruby answers `"gem label"` for `Shelf::Child.new.label`.
#[allow(dead_code)]
pub fn inherited_respond_app() -> emit_and_run::Overlay {
    with_gem_lib(SHELF_KINDS)
        .write("config/initializers/shelf_kinds.rb", KINDS_REOPEN)
        .write("app/models/kinds_probe.rb", "class KindsProbe\n  def self.describe\n    Shelf::Child.new.label\n  end\nend\n")
}

/// A gem module a model includes for its `to_param`.
pub const SHELF_SLUG: &str = r##"module Shelf
  module Slug
    def to_param
      "gem-#{id}"
    end
  end
end
"##;

/// An initializer's reopen of it, `to_param` written under `{guard}` (empty for none).
const SLUG_REOPEN: &str = r##"module Shelf
  module Slug
    {guard}
    def to_param
      "app-#{id}"
    end
    {end}
  end
end
"##;

/// `Article` including the gem's `Shelf::Slug`, which the initializer
/// reopens with its own `to_param`, written under `guard` when one is given.
fn slug_reopen_app(guard: Option<&str>) -> emit_and_run::Overlay {
    let reopen = SLUG_REOPEN.replace("{guard}", guard.unwrap_or("")).replace("{end}", if guard.is_some() { "end" } else { "" });
    with_gem_lib(SHELF_SLUG)
        .write("config/initializers/shelf_slug.rb", &reopen)
        .edit("app/models/article.rb", "class Article < ApplicationRecord\n", "class Article < ApplicationRecord\n  include Shelf::Slug\n")
}

/// Ruby answers `"app-7"` for `Article.new(id: 7).to_param`.
#[allow(dead_code)]
pub fn slug_reopen_plain_app() -> emit_and_run::Overlay {
    slug_reopen_app(None)
}

/// The reopen's `to_param` under `if ENV["SHELF_SLUG"]`, which only running
/// the program answers. Ruby answers `"gem-7"` with the variable unset and
/// `"app-7"` with it set.
#[allow(dead_code)]
pub fn slug_reopen_env_app() -> emit_and_run::Overlay {
    slug_reopen_app(Some("if ENV[\"SHELF_SLUG\"]"))
}

pub const SLUG_ASSERTIONS: &str = r##"got = Article.new(id: 7).to_param
raise "to_param #{got.inspect}" unless got == "{expected}"
puts "slug passed"
"##;

/// One reopen defining `to_param` unconditionally, then again under
/// `if ENV["SHELF_SLUG"]`. Ruby answers `"app-7"` for
/// `Article.new(id: 7).to_param` with the variable unset and `"env-7"`
/// with it set.
#[allow(dead_code)]
pub fn slug_reopen_twice_app() -> emit_and_run::Overlay {
    let reopen = "module Shelf\n  module Slug\n    def to_param\n      \"app-#{id}\"\n    end\n\n    if ENV[\"SHELF_SLUG\"]\n      def to_param\n        \"env-#{id}\"\n      end\n    end\n  end\nend\n";
    with_gem_lib(SHELF_SLUG)
        .write("config/initializers/shelf_slug.rb", reopen)
        .edit("app/models/article.rb", "class Article < ApplicationRecord\n", "class Article < ApplicationRecord\n  include Shelf::Slug\n")
}

/// A reopen whose `to_param` reads `PREFIX`, a constant only the gem's
/// opening declares. Ruby answers `"slug-7"` for `Article.new(id: 7).to_param`.
#[allow(dead_code)]
pub fn slug_reopen_prefix_app() -> emit_and_run::Overlay {
    let gem = SHELF_SLUG.replace("  module Slug\n", "  module Slug\n    PREFIX = \"slug\"\n\n");
    let reopen = "module Shelf\n  module Slug\n    def to_param\n      \"#{PREFIX}-#{id}\"\n    end\n  end\nend\n";
    with_gem_lib(&gem)
        .write("config/initializers/shelf_slug.rb", reopen)
        .edit("app/models/article.rb", "class Article < ApplicationRecord\n", "class Article < ApplicationRecord\n  include Shelf::Slug\n")
}
