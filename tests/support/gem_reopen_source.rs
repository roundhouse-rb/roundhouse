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
