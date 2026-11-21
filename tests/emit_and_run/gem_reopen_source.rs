//! A source gem's class that the app reopens keeps the gem's methods
//! (#2093). See `tests/support/gem_reopen_source.rs`.

use roundhouse::build_target::BuildTarget;

use super::gem_reopen_source_contract as contract;

/// The Ruby family, with the gem listed in `source_gems` so the tree
/// carries it: the gem's methods and the app's, the app's `label` winning.
#[test]
fn a_reopened_gem_class_answers_the_gems_methods_and_the_apps() {
    contract::listed(contract::local_app()).run_ruby(contract::LOCAL_ASSERTIONS).assert_passes();
}

#[test]
fn a_gem_class_reopened_for_a_constant_and_an_include_answers_all_three() {
    contract::listed(contract::extras_app()).run_ruby(contract::EXTRAS_ASSERTIONS).assert_passes();
}

/// A reopen at the gem class's autoload path is a file Rails never loads:
/// the class keeps the gem's methods and gains none of the file's.
#[test]
fn a_reopen_at_the_gem_classs_autoload_path_leaves_the_gems_class() {
    contract::listed(contract::models_app()).run_ruby(contract::MODELS_ASSERTIONS).assert_passes();
}

/// A boot file that requires the reopen by path loads it, autoload path or not.
#[test]
fn a_reopen_at_the_gem_classs_autoload_path_an_initializer_requires_is_loaded() {
    contract::listed(contract::required_models_app()).run_ruby(contract::REQUIRED_MODELS_ASSERTIONS).assert_passes();
}

/// `Rails.root.join("app", "models", "shelf", "box")` names the same file.
#[test]
fn a_reopen_at_the_gem_classs_autoload_path_required_by_joined_parts_is_loaded() {
    contract::listed(contract::split_required_models_app()).run_ruby(contract::REQUIRED_MODELS_ASSERTIONS).assert_passes();
}

/// A file the boot loads through another file's `require_relative` is loaded too.
#[test]
fn a_reopen_at_the_gem_classs_autoload_path_required_through_a_lib_file_is_loaded() {
    contract::listed(contract::chain_required_models_app()).run_ruby(contract::REQUIRED_MODELS_ASSERTIONS).assert_passes();
}

/// A require in a comment loads nothing.
#[test]
fn a_reopen_at_the_gem_classs_autoload_path_required_only_in_a_comment_leaves_the_gems_class() {
    contract::listed(contract::commented_required_models_app()).run_ruby(contract::MODELS_ASSERTIONS).assert_passes();
}

/// A constant the gem assigns is defined as one it declares is.
#[test]
fn a_reopen_at_the_autoload_path_of_a_constant_the_gem_assigns_leaves_the_gems_class() {
    contract::listed(contract::assigned_models_app()).run_ruby(contract::MODELS_ASSERTIONS).assert_passes();
}

/// The path is camelized with the app's acronyms, as Zeitwerk does.
#[test]
fn a_reopen_at_an_acronym_autoload_path_leaves_the_gems_class() {
    contract::listed(contract::acronym_models_app()).run_ruby(contract::ACRONYM_MODELS_ASSERTIONS).assert_passes();
}

/// A gem file that nothing requires defines nothing, so Rails loads the
/// app's file at that path. The gem's file isn't one roundhouse reads
/// either: a source gem is the files its requires reach.
#[test]
fn a_reopen_at_the_autoload_path_of_a_gem_class_in_an_unrequired_file_is_loaded() {
    contract::listed(contract::unrequired_gem_box_models_app()).run_ruby(contract::APP_FILE_MODELS_ASSERTIONS).assert_passes();
}

/// Nor does a gem file the gem autoloads by feature name: Zeitwerk sets
/// its own autoload over that one, from the app's file.
#[test]
fn a_reopen_at_the_autoload_path_of_a_gem_class_autoloaded_by_feature_is_loaded() {
    contract::listed(contract::feature_autoloaded_gem_box_models_app()).run_ruby(contract::AUTOLOADED_MODELS_ASSERTIONS).assert_passes();
}

/// The emitted `Shelf::Box`: one file holding each opening in load order.
fn shelf_box(app: super::emit_and_run::Overlay) -> String {
    let (emitted, _errors) = app.emit(BuildTarget::Spinel);
    std::fs::read_to_string(emitted.join("app/models/shelf/box.rb")).expect("app/models/shelf/box.rb")
        .lines()
        .filter(|l| !l.starts_with("#<SPINEL_SOURCE>"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// On Spinel the gem is compiled from source because the app names
/// `Shelf::Box`, even though the app's own reopen also declares it: the
/// gem's opening comes first and the app's after it, so the app's `label`
/// is the one that stands, as in Ruby.
#[test]
fn the_spinel_emit_compiles_the_gem_class_the_app_reopens() {
    let source = shelf_box(contract::local_app());
    let at = |needle: &str| source.find(needle).unwrap_or_else(|| panic!("{needle:?} in\n{source}"));
    let (kind, local) = (at("def self.kind"), at("def self.local_tag"));
    let gem_label = at("\"shelf box\"");
    let app_label = at("\"app box\"");
    assert!(kind < local && gem_label < app_label, "{source}");
}

/// A reopen that adds only a constant and an `include` brings the gem in too.
/// (Only what the app reaches is kept: `label`, which nothing calls, is not.)
#[test]
fn the_spinel_emit_compiles_the_gem_class_a_constant_only_reopen_names() {
    let source = shelf_box(contract::extras_app());
    for needle in ["def self.kind", "LIMIT = 3", "include Tagging"] {
        assert!(source.contains(needle), "{needle:?} in\n{source}");
    }
    assert!(source.find("def self.kind").unwrap() < source.find("LIMIT = 3").unwrap(), "{source}");
}

/// A reopen written in an initializer, run on the Ruby family with the gem
/// listed: the gem's methods and the initializer's, the initializer's
/// `label` winning.
#[test]
fn a_gem_class_reopened_in_an_initializer_answers_the_gems_methods_and_the_apps() {
    contract::listed(contract::initializer_app()).run_ruby(contract::LOCAL_ASSERTIONS).assert_passes();
}

/// On Spinel the initializer's opening is emitted after the gem's, in the
/// same file, in the nested form and in the compact one.
#[test]
fn the_spinel_emit_keeps_a_gem_class_reopened_in_an_initializer() {
    for app in [contract::initializer_app(), contract::compact_initializer_app()] {
        let source = shelf_box(app);
        let at = |needle: &str| source.find(needle).unwrap_or_else(|| panic!("{needle:?} in\n{source}"));
        let (kind, local) = (at("def self.kind"), at("def self.local_tag"));
        let gem_label = at("\"shelf box\"");
        let app_label = at("\"app box\"");
        assert!(kind < local && gem_label < app_label, "{source}");
    }
}

/// The `class_eval` form, run on the Ruby family with the gem listed.
#[test]
fn a_gem_class_class_evaled_in_an_initializer_answers_the_gems_methods_and_the_apps() {
    contract::listed(contract::class_eval_app()).run_ruby(contract::LOCAL_ASSERTIONS).assert_passes();
}

/// On Spinel a `class_eval` block of `def`s on a gem's class is one more
/// opening of it, after the gem's, and the initializer statement is no
/// longer reported as having no effect.
#[test]
fn the_spinel_emit_keeps_a_gem_class_class_evaled_in_an_initializer() {
    let unconsumed = |app: super::emit_and_run::Overlay| -> Vec<String> {
        app.ingest_diagnostics()
            .into_iter()
            .filter(|d| matches!(d.kind, roundhouse::diagnostic::DiagnosticKind::UnconsumedInitializer { .. }))
            .map(|d| d.message)
            .collect()
    };
    let source = shelf_box(contract::class_eval_app());
    let at = |needle: &str| source.find(needle).unwrap_or_else(|| panic!("{needle:?} in\n{source}"));
    let (kind, local) = (at("def self.kind"), at("def self.local_tag"));
    let gem_label = at("\"shelf box\"");
    let app_label = at("\"app box\"");
    assert!(kind < local && gem_label < app_label, "{source}");
    let warned = unconsumed(contract::class_eval_app());
    assert!(!warned.iter().any(|m| m.contains("Shelf::Box.class_eval")), "{warned:#?}");
}

/// A block after the initializer's own reopen in the same file is the
/// later opening: its `label` stands (Ruby: `box / app box / local`).
#[test]
fn a_class_eval_after_a_reopen_in_the_same_initializer_stands() {
    contract::listed(contract::reopen_then_class_eval_app()).run_ruby(contract::LOCAL_ASSERTIONS).assert_passes();
}

/// The same with the reopen in an initializer that sorts first.
#[test]
fn a_class_eval_in_a_later_initializer_stands_over_an_earlier_reopen() {
    contract::listed(contract::reopen_then_class_eval_across_files_app()).run_ruby(contract::LOCAL_ASSERTIONS).assert_passes();
}

/// A helper module the initializer defines, called only from a kept
/// block's method, is kept with it.
#[test]
fn a_helper_a_kept_class_eval_calls_is_kept() {
    contract::listed(contract::class_eval_helper_app()).run_ruby(contract::HELPER_ASSERTIONS).assert_passes();
}

/// A helper that a kept helper calls is kept too.
#[test]
fn a_helper_a_kept_helper_calls_is_kept() {
    contract::listed(contract::class_eval_transitive_helper_app()).run_ruby(contract::TRANSITIVE_ASSERTIONS).assert_passes();
}

/// A helper a gem class's reopen in the initializer calls is kept.
#[test]
fn a_helper_an_initializer_reopen_calls_is_kept() {
    contract::listed(contract::reopen_helper_app()).run_ruby(contract::TRANSITIVE_ASSERTIONS).assert_passes();
}

/// A helper a kept helper's parameter default calls is kept.
#[test]
fn a_helper_a_parameter_default_calls_is_kept() {
    contract::listed(contract::class_eval_default_helper_app()).run_ruby(contract::TRANSITIVE_ASSERTIONS).assert_passes();
}

/// A helper a kept helper's constant value calls is kept.
#[test]
fn a_helper_a_constant_value_calls_is_kept() {
    contract::listed(contract::class_eval_constant_helper_app()).run_ruby(contract::TRANSITIVE_ASSERTIONS).assert_passes();
}

/// A helper module a kept helper includes is kept.
#[test]
fn a_helper_a_kept_helper_includes_is_kept() {
    contract::listed(contract::class_eval_included_helper_app()).run_ruby(contract::TRANSITIVE_ASSERTIONS).assert_passes();
}

/// A helper module a kept helper extends is kept.
#[test]
fn a_helper_a_kept_helper_extends_is_kept() {
    contract::listed(contract::class_eval_extended_helper_app()).run_ruby(contract::TRANSITIVE_ASSERTIONS).assert_passes();
}

/// A helper class a kept helper inherits from is kept.
#[test]
fn a_helper_a_kept_helper_inherits_from_is_kept() {
    contract::listed(contract::class_eval_superclass_helper_app()).run_ruby(contract::TRANSITIVE_ASSERTIONS).assert_passes();
}

/// A helper a kept helper's class ivar initializer calls is kept.
#[test]
fn a_helper_a_kept_helpers_class_ivar_calls_is_kept() {
    contract::listed(contract::class_eval_class_ivar_helper_app()).run_ruby(contract::TRANSITIVE_ASSERTIONS).assert_passes();
}

/// A helper the block names from the top level (`::LabelHelper`) is kept.
#[test]
fn a_helper_a_kept_class_eval_names_from_the_top_level_is_kept() {
    contract::listed(contract::class_eval_rooted_helper_app()).run_ruby(contract::HELPER_ASSERTIONS).assert_passes();
}

/// A block with a `def` on another receiver is not taken: that method is
/// the other object's. It stays reported, and the class is the gem's.
#[test]
fn a_class_eval_defining_on_another_receiver_is_left_reported() {
    let source = shelf_box(contract::class_eval_other_receiver_app());
    assert!(!source.contains("local_tag") && !source.contains("\"other\""), "{source}");
    let warned: Vec<String> = contract::class_eval_other_receiver_app()
        .ingest_diagnostics()
        .into_iter()
        .filter(|d| matches!(d.kind, roundhouse::diagnostic::DiagnosticKind::UnconsumedInitializer { .. }))
        .map(|d| d.message)
        .collect();
    assert!(warned.iter().any(|m| m.contains("Shelf::Box.class_eval")), "{warned:#?}");
}

/// A block whose method reads a bare constant the gem's class defines is
/// not taken: in Ruby the name is the top level's, and the opening would
/// put it in the class. It stays reported, and the class is the gem's.
#[test]
fn a_class_eval_reading_a_constant_of_the_gems_class_is_left_reported() {
    let source = shelf_box(contract::class_eval_bare_constant_app());
    assert!(!source.contains("local_tag"), "{source}");
    let warned: Vec<String> = contract::class_eval_bare_constant_app()
        .ingest_diagnostics()
        .into_iter()
        .filter(|d| matches!(d.kind, roundhouse::diagnostic::DiagnosticKind::UnconsumedInitializer { .. }))
        .map(|d| d.message)
        .collect();
    assert!(warned.iter().any(|m| m.contains("Shelf::Box.class_eval")), "{warned:#?}");
}

/// A block whose method reads a bare name the gem's class inherits, from
/// its superclass or a module it includes, is not taken either: Ruby
/// reads the initializer's top-level `TokenSource` there, and the opening
/// would read the ancestor's. It stays reported, and the class is the gem's.
#[test]
fn a_class_eval_reading_a_constant_the_gems_class_inherits_is_left_reported() {
    for app in contract::class_eval_inherited_constant_apps() {
        let source = shelf_box(app);
        assert!(!source.contains("local_tag"), "{source}");
    }
    for app in contract::class_eval_inherited_constant_apps() {
        let warned: Vec<String> = app
            .ingest_diagnostics()
            .into_iter()
            .filter(|d| matches!(d.kind, roundhouse::diagnostic::DiagnosticKind::UnconsumedInitializer { .. }))
            .map(|d| d.message)
            .collect();
        assert!(warned.iter().any(|m| m.contains("Shelf::Box.class_eval")), "{warned:#?}");
    }
}

/// A block whose method reads a bare name an initializer reopen defines
/// in the gem's class is not taken either: Ruby reads the top level's
/// `TokenSource` there, and the opening would read the reopen's. It stays
/// reported.
#[test]
fn a_class_eval_reading_a_constant_an_initializer_reopen_defines_is_left_reported() {
    let source = shelf_box(contract::class_eval_reopen_constant_app());
    assert!(!source.contains("local_tag"), "{source}");
    let warned: Vec<String> = contract::class_eval_reopen_constant_app()
        .ingest_diagnostics()
        .into_iter()
        .filter(|d| matches!(d.kind, roundhouse::diagnostic::DiagnosticKind::UnconsumedInitializer { .. }))
        .map(|d| d.message)
        .collect();
    assert!(warned.iter().any(|m| m.contains("Shelf::Box.class_eval")), "{warned:#?}");
}

/// A block whose method reads a class variable is not taken: Ruby looks
/// `@@configured` up in the block's lexical scope, the top level, and the
/// opening would read the gem class's. It stays reported.
#[test]
fn a_class_eval_reading_a_class_variable_is_left_reported() {
    let source = shelf_box(contract::class_eval_class_variable_app());
    assert!(!source.contains("local_tag"), "{source}");
    let warned: Vec<String> = contract::class_eval_class_variable_app()
        .ingest_diagnostics()
        .into_iter()
        .filter(|d| matches!(d.kind, roundhouse::diagnostic::DiagnosticKind::UnconsumedInitializer { .. }))
        .map(|d| d.message)
        .collect();
    assert!(warned.iter().any(|m| m.contains("Shelf::Box.class_eval")), "{warned:#?}");
}

/// A block the ingest refuses leaves the file's other blocks taken.
#[test]
fn a_refused_class_eval_block_leaves_the_files_others_kept() {
    contract::listed(contract::refused_then_class_eval_app()).run_ruby(contract::LOCAL_ASSERTIONS).assert_passes();
}

/// A block with any statement besides `def`s is not taken: it stays
/// reported, and the emitted class is the gem's alone.
#[test]
fn a_class_eval_block_that_writes_a_constant_is_left_reported() {
    let source = shelf_box(contract::class_eval_limit_app());
    assert!(!source.contains("local_tag"), "{source}");
    let warned: Vec<String> = contract::class_eval_limit_app()
        .ingest_diagnostics()
        .into_iter()
        .filter(|d| matches!(d.kind, roundhouse::diagnostic::DiagnosticKind::UnconsumedInitializer { .. }))
        .map(|d| d.message)
        .collect();
    assert!(warned.iter().any(|m| m.contains("Shelf::Box.class_eval")), "{warned:#?}");
}

/// A controller including a gem module the initializer reopens gets both
/// openings' methods, and the reopen's `label`, as in Ruby
/// (`"gem / local / app label"`): with the reopen written in the
/// initializer, and in `lib/` (#2122's form).
#[test]
fn a_controller_including_a_reopened_gem_module_gets_both_openings() {
    for app in [contract::included_module_app(), contract::included_module_lib_app()] {
        let (emitted, _errors) = app.emit(BuildTarget::Spinel);
        let source = std::fs::read_to_string(emitted.join("app/controllers/shelves_controller.rb")).expect("shelves_controller.rb");
        for needle in ["def original", "def local", "\"app label\""] {
            assert!(source.contains(needle), "{needle:?} in\n{source}");
        }
        assert!(!source.contains("\"gem label\""), "{source}");
    }
}

/// A gem module whose own opening is empty is still the gem's, so the
/// initializer's opening is kept.
#[test]
fn a_gem_module_declared_empty_keeps_the_initializers_methods() {
    let source = std::fs::read_to_string(contract::empty_module_app().emit(BuildTarget::Spinel).0.join("app/models/shelf/hooks.rb"))
        .expect("app/models/shelf/hooks.rb");
    assert!(source.contains("def self.hook"), "{source}");
}

/// A gem module whose only member is an `attr_reader` is still the gem's,
/// so the initializer's opening is kept.
#[test]
fn a_gem_module_with_only_an_attr_reader_keeps_the_initializers_methods() {
    let source = std::fs::read_to_string(contract::attr_module_app().emit(BuildTarget::Spinel).0.join("app/models/shelf/box.rb"))
        .expect("app/models/shelf/box.rb");
    assert!(source.contains("def self.local_tag"), "{source}");
}

/// The emitted `ShelvesController`.
fn shelves_controller(app: super::emit_and_run::Overlay) -> String {
    let (emitted, _errors) = app.emit(BuildTarget::Spinel);
    std::fs::read_to_string(emitted.join("app/controllers/shelves_controller.rb")).expect("shelves_controller.rb")
}

/// A `label` the initializer defines under a false load-time `if` does not
/// replace the gem's: Ruby answers `"gem / local / gem label"`.
#[test]
fn a_conditional_reopen_does_not_replace_the_gems_method_in_the_controller() {
    let source = shelves_controller(contract::conditional_reopen_app());
    for needle in ["def original", "def local", "\"gem label\""] {
        assert!(source.contains(needle), "{needle:?} in\n{source}");
    }
    assert!(!source.contains("\"app label\""), "{source}");
}

/// With the constant true Ruby answers `"gem / local / app label"`, but a
/// constant's value is not known where the reopen is spliced: a `label`
/// defined under it does not replace the gem's.
#[test]
fn a_conditional_reopen_on_a_constants_value_keeps_the_gems_method() {
    let source = shelves_controller(contract::true_conditional_reopen_app());
    assert!(source.contains("\"gem label\""), "{source}");
}

/// The later of two `label`s in one opening is the one that stands, as in
/// Ruby (`"gem / local / app label"`).
#[test]
fn the_later_of_two_definitions_in_one_reopen_stands_in_the_controller() {
    let source = shelves_controller(contract::twice_reopen_app());
    assert!(source.contains("\"app label\""), "{source}");
    assert!(!source.contains("\"first label\"") && !source.contains("\"gem label\""), "{source}");
}

/// The gem's opening comes first in the splice even when the app's is
/// read before it: Ruby answers `"gem / local / app label"`.
#[test]
fn a_reopen_read_before_the_gems_opening_still_replaces_its_method_in_the_controller() {
    let source = shelves_controller(contract::models_reopen_app());
    for needle in ["def original", "def local", "\"app label\""] {
        assert!(source.contains(needle), "{needle:?} in\n{source}");
    }
    assert!(!source.contains("\"gem label\""), "{source}");
}

/// A `label` defined under `if defined?(Shelf::Tagging)`, a constant the
/// program carries, replaces the gem's: Ruby answers
/// `"gem / local / app label"`.
#[test]
fn a_reopen_defining_a_method_when_a_carried_constant_is_defined_replaces_the_gems() {
    let source = shelves_controller(contract::defined_reopen_app());
    for needle in ["def original", "def local", "\"app label\""] {
        assert!(source.contains(needle), "{needle:?} in\n{source}");
    }
    assert!(!source.contains("\"gem label\""), "{source}");
}

/// Under `unless defined?(Shelf::Tagging)` the reopen's `label` is never
/// defined: Ruby answers `"gem / local / gem label"`.
#[test]
fn a_reopen_defining_a_method_unless_a_carried_constant_is_defined_keeps_the_gems() {
    let source = shelves_controller(contract::undefined_reopen_app());
    for needle in ["def original", "def local", "\"gem label\""] {
        assert!(source.contains(needle), "{needle:?} in\n{source}");
    }
    assert!(!source.contains("\"app label\""), "{source}");
}

/// Under `if ENV["SHELF_LABEL"]` only running the program says whether the
/// reopen's `label` is defined, so the splice keeps the gem's: what Ruby
/// answers with the variable unset (`"gem / local / gem label"`).
#[test]
fn a_reopen_defining_a_method_under_an_unknown_condition_keeps_the_gems() {
    let source = shelves_controller(contract::env_reopen_app());
    for needle in ["def original", "def local", "\"gem label\""] {
        assert!(source.contains(needle), "{needle:?} in\n{source}");
    }
    assert!(!source.contains("\"app label\""), "{source}");
}

/// `respond_to?(:enabled)` after `def self.enabled` in an arm Ruby skips
/// (`unless defined?(Shelf::Tagging)`) is false, so the reopen's `label`
/// is never defined: Ruby answers `"gem / local / gem label"`.
#[test]
fn a_respond_to_after_a_skipped_arms_class_method_keeps_the_gems() {
    let source = shelves_controller(contract::respond_after_skipped_app());
    for needle in ["def original", "def local", "\"gem label\""] {
        assert!(source.contains(needle), "{needle:?} in\n{source}");
    }
    assert!(!source.contains("\"app label\""), "{source}");
}

/// After `def self.enabled` under `if ENV["SHELF_ENABLED"]`, only running
/// the program says what `respond_to?(:enabled)` answers, so the splice
/// keeps the gem's `label`: what Ruby answers with the variable unset
/// (`"gem / local / gem label"`).
#[test]
fn a_respond_to_after_an_unknown_arms_class_method_keeps_the_gems() {
    let source = shelves_controller(contract::respond_after_unknown_app());
    for needle in ["def original", "def local", "\"gem label\""] {
        assert!(source.contains(needle), "{needle:?} in\n{source}");
    }
    assert!(!source.contains("\"app label\""), "{source}");
}

/// After `def self.enabled` under `if defined?(Shelf::Tagging)`,
/// `respond_to?(:enabled)` is true and the reopen's `label` replaces the
/// gem's: Ruby answers `"gem / local / app label"`.
#[test]
fn a_respond_to_after_a_settled_arms_class_method_replaces_the_gems() {
    let source = shelves_controller(contract::respond_after_settled_app());
    for needle in ["def original", "def local", "\"app label\""] {
        assert!(source.contains(needle), "{needle:?} in\n{source}");
    }
    assert!(!source.contains("\"gem label\""), "{source}");
}

/// A `respond_to?(:enabled)` written before the settled arm that defines
/// `self.enabled` runs before it: Ruby answers `"gem / local / gem label"`.
#[test]
fn a_respond_to_before_a_settled_arms_class_method_keeps_the_gems() {
    let source = shelves_controller(contract::respond_before_settled_app());
    for needle in ["def original", "def local", "\"gem label\""] {
        assert!(source.contains(needle), "{needle:?} in\n{source}");
    }
    assert!(!source.contains("\"app label\""), "{source}");
}

/// A gem class whose only member is a class instance variable
/// (`@configured = true`) is still the gem's, so the initializer's opening
/// is kept: Ruby answers `"local"`.
#[test]
fn a_gem_class_with_only_a_class_ivar_keeps_the_initializers_methods() {
    let source = std::fs::read_to_string(contract::ivar_class_app().emit(BuildTarget::Spinel).0.join("app/models/shelf/box.rb"))
        .expect("app/models/shelf/box.rb");
    assert!(source.contains("def self.local_tag"), "{source}");
}

/// `Child`'s `if respond_to?(:enabled)` runs before the later reopen of its
/// superclass defines `self.enabled` in the same file, so Ruby answers
/// `"gem label"`. The guard is not settled from that later definition: the
/// emitted `Child` keeps it. (A Ruby run of the tree can't show this: each
/// class's openings are written to its own file, and `child.rb` requires
/// `base.rb`, so the emitted program defines `enabled` first.)
#[test]
fn a_respond_to_before_a_superclasss_class_method_in_the_same_file_is_not_settled() {
    let (emitted, _errors) = contract::listed(contract::inherited_respond_app()).emit(BuildTarget::Ruby);
    let source = std::fs::read_to_string(emitted.join("app/models/shelf/child.rb")).expect("app/models/shelf/child.rb");
    assert!(source.contains("if respond_to?(:enabled)"), "{source}");
}

/// The emitted `Article`'s copy of the `to_param` its included module defines.
fn article_to_param(app: super::emit_and_run::Overlay) -> String {
    let (emitted, _errors) = contract::listed(app).emit(BuildTarget::Spinel);
    let source = std::fs::read_to_string(emitted.join("app/models/article.rb")).expect("app/models/article.rb");
    let at = source.find("def to_param").unwrap_or_else(|| panic!("def to_param in\n{source}"));
    source[at..].lines().take(3).collect::<Vec<_>>().join("\n")
}

/// The initializer's reopen of a gem module redefines the `to_param` a
/// model takes from it: Ruby answers `"app-7"`, and the model's copy is
/// the reopen's.
#[test]
fn a_model_takes_to_param_from_the_initializers_reopen_of_a_gem_module() {
    let to_param = article_to_param(contract::slug_reopen_plain_app());
    assert!(to_param.contains("app-") && !to_param.contains("gem-"), "{to_param}");
    contract::listed(contract::slug_reopen_plain_app())
        .run_ruby(&contract::SLUG_ASSERTIONS.replace("{expected}", "app-7"))
        .assert_passes();
}

/// A reopen's `to_param` under `if ENV["SHELF_SLUG"]` is not known to be
/// defined, so the gem's stays the model's: Ruby answers `"gem-7"` with
/// the variable unset.
#[test]
fn a_model_keeps_the_gems_to_param_over_a_reopens_unsettled_one() {
    let to_param = article_to_param(contract::slug_reopen_env_app());
    assert!(to_param.contains("gem-") && !to_param.contains("app-"), "{to_param}");
    contract::listed(contract::slug_reopen_env_app())
        .run_ruby(&contract::SLUG_ASSERTIONS.replace("{expected}", "gem-7"))
        .assert_passes();
}

/// An unconditional `to_param`, then one under `if ENV["SHELF_SLUG"]`, in
/// the same reopen: only running the program answers the second, so the
/// first stands over the gem's. Ruby answers `"app-7"` with the variable unset.
#[test]
fn a_model_takes_a_reopens_unconditional_to_param_over_its_later_unsettled_one() {
    let to_param = article_to_param(contract::slug_reopen_twice_app());
    assert!(to_param.contains("app-") && !to_param.contains("gem-"), "{to_param}");
    contract::listed(contract::slug_reopen_twice_app())
        .run_ruby(&contract::SLUG_ASSERTIONS.replace("{expected}", "app-7"))
        .assert_passes();
}

/// The reopen's `to_param` reads `PREFIX`, which the gem's opening of the
/// module declares: the model's copy names the module's constant, and
/// Ruby answers `"slug-7"`.
#[test]
fn a_models_to_param_from_a_reopen_reads_the_gems_constant() {
    contract::listed(contract::slug_reopen_prefix_app())
        .run_ruby(&contract::SLUG_ASSERTIONS.replace("{expected}", "slug-7"))
        .assert_passes();
}
