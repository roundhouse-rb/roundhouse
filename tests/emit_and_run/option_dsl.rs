use super::{emit_and_run, option_dsl};

#[test]
fn an_extended_option_dsl_defines_the_readers_and_the_builders_writers() {
    let run = emit_and_run::real_blog()
        .write("app/models/settings/option.rb", option_dsl::OPTION)
        .write("app/models/settings/config.rb", &option_dsl::config(option_dsl::LITERAL_OPTIONS, ""))
        .run_ruby(option_dsl::LITERAL_ASSERTIONS);
    run.assert_passes();
    assert!(run.stdout.contains("option DSL passed"), "{}", run.stdout);
    let config = std::fs::read_to_string(run.emitted.join("app/models/settings/config.rb")).expect("emitted config");
    assert!(config.contains("def realm_name(*_args)") && config.contains("def realm(*args, &block)"), "{config}");
    assert!(!config.contains("define_method"), "{config}");
}

/// doorkeeper's `base_controller`: a lambda default the config runs with `instance_exec`, so it reads the config's own options.
#[test]
fn a_lambda_default_runs_on_the_config_that_reads_it() {
    let run = emit_and_run::real_blog()
        .write("app/models/settings/option.rb", option_dsl::OPTION)
        .write("app/models/settings/config.rb", &option_dsl::config(option_dsl::RESOLVING_OPTIONS, option_dsl::RESOLVE))
        .run_ruby(
            r##"raise "default" unless Settings::Config::Builder.new.build.resolve_controller(:base) == "ActionController::Base"
raise "api" unless Settings.configure { api_only true }.resolve_controller(:base) == "ActionController::API"
raise "own" unless Settings.configure { base_controller "ApplicationController" }.resolve_controller(:base) == "ApplicationController"
puts "lambda default passed"
"##,
        );
    run.assert_passes();
    assert!(run.stdout.contains("lambda default passed"), "{}", run.stdout);
}

fn configured_superclass_app(initializer: Option<&str>) -> emit_and_run::Overlay {
    let mut overlay = emit_and_run::real_blog()
        .write("app/models/settings/option.rb", option_dsl::OPTION)
        .write("app/models/settings/config.rb", &option_dsl::config(option_dsl::RESOLVING_OPTIONS, option_dsl::RESOLVE_CLASS))
        .edit(
            "app/controllers/application_controller.rb",
            "  stale_when_importmap_changes\n",
            "  stale_when_importmap_changes\n\n  private\n\n  def app_marker\n    \"app\"\n  end\n",
        )
        .write("app/controllers/settings_base_controller.rb", "class SettingsBaseController < Settings.config.resolve_controller(:base)\nend\n")
        .write(
            "app/controllers/pings_controller.rb",
            "class PingsController < SettingsBaseController\n  def show\n    render plain: \"#{SettingsBaseController.superclass.name} #{respond_to?(:app_marker, true)}\"\n  end\nend\n",
        )
        .edit("config/routes.rb", "  root \"articles#index\"\n", "  root \"articles#index\"\n  get \"/ping\", to: \"pings#show\"\n");
    if let Some(body) = initializer {
        overlay = overlay.write("config/initializers/settings.rb", body);
    }
    overlay
}

fn ping_test(expected: &str) -> String {
    format!(
        "require \"test_helper\"\n\nclass PingsControllerTest < ActionDispatch::IntegrationTest\n  test \"the parent is the one the configuration names\" do\n    get \"/ping\"\n    assert_response :success\n    assert_equal {expected:?}, response.body\n  end\nend\n"
    )
}

/// doorkeeper's `class ApplicationController < Doorkeeper.config.resolve_controller(:base)`: the host's
/// initializer has configured the option by the time the class loads, so the parent is the one it names.
#[test]
fn a_superclass_read_off_the_configured_object_is_the_one_the_initializer_names() {
    configured_superclass_app(Some("Settings.configure do\n  base_controller \"ApplicationController\"\nend\n"))
        .write("test/controllers/pings_controller_test.rb", &ping_test("ApplicationController true"))
        .run_test("test/controllers/pings_controller_test.rb")
        .assert_passes();
}

/// With no initializer, the option's lambda default answers, run on the config as `instance_exec` runs it.
#[test]
fn a_superclass_read_off_an_unconfigured_object_is_its_default() {
    configured_superclass_app(None)
        .write("test/controllers/pings_controller_test.rb", &ping_test("ActionController::Base false"))
        .run_test("test/controllers/pings_controller_test.rb")
        .assert_passes();
}

/// What the option holds is known only when the app boots (`ENV.fetch`): the superclass stays refused.
#[test]
fn a_superclass_from_a_value_known_only_at_boot_stays_refused() {
    let errors = configured_superclass_app(Some("Settings.configure do\n  base_controller ENV.fetch(\"BASE_CONTROLLER\", \"ApplicationController\")\nend\n")).check_errors();
    assert!(errors.iter().any(|e| e.contains("superclass is an expression")), "{errors:?}");
}

/// doorkeeper's ORM mixins: `included do self.table_name = compute_doorkeeper_table_name end`, where the
/// class method builds the name from ActiveRecord's `pluralize_table_names` and prefixes. The model reads
/// the table the method names, here real-blog's `articles`, not the `notes` its class name would give.
#[test]
fn a_table_name_a_concern_computes_binds_the_models_columns() {
    let run = emit_and_run::real_blog()
        .write(
            "app/models/concerns/computed_table.rb",
            "module ComputedTable\n  extend ActiveSupport::Concern\n\n  included do\n    self.table_name = compute_table_name\n  end\n\n  class_methods do\n    def compute_table_name\n      table_name = \"article\"\n      table_name = table_name.pluralize if pluralize_table_names\n      \"#{table_name_prefix}#{table_name}#{table_name_suffix}\"\n    end\n  end\nend\n",
        )
        .write("app/models/note.rb", "class Note < ApplicationRecord\n  include ComputedTable\nend\n")
        .run_ruby(
            r##"Article.create!(title: "Computed", body: "A body long enough to pass the validation")
raise "count: #{Note.count}" unless Note.count == Article.count
raise "title: #{Note.first.title.inspect}" unless Note.first.title == "Computed"
puts "computed table passed"
"##,
        );
    run.assert_passes();
    assert!(run.stdout.contains("computed table passed"), "{}", run.stdout);
}


/// doorkeeper's `has_many :authorized_tokens, class_name: Doorkeeper.config.access_token_class.to_s`: the
/// association reads the class the configured option names, not the `Remark` its own name would give.
#[test]
fn an_association_class_name_read_off_the_configured_object_is_the_one_it_names() {
    let run = emit_and_run::real_blog()
        .write("app/models/settings/option.rb", option_dsl::OPTION)
        .write("app/models/settings/config.rb", &option_dsl::config("    option :remark_class, default: \"Comment\"", ""))
        .edit(
            "app/models/article.rb",
            "class Article < ApplicationRecord\n",
            "class Article < ApplicationRecord\n  has_many :remarks, class_name: Settings.config.remark_class.to_s, foreign_key: :article_id\n",
        )
        .run_ruby(
            r##"article = Article.create!(title: "Configured", body: "A body long enough to pass the validation")
Comment.create!(article_id: article.id, commenter: "Ann", body: "First")
raise "remarks: #{article.remarks.map(&:class).inspect}" unless article.remarks.count == 1 && article.remarks.first.is_a?(Comment)
puts "configured association passed"
"##,
        );
    run.assert_passes();
    assert!(run.stdout.contains("configured association passed"), "{}", run.stdout);
}
