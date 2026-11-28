//! A class that includes the url helpers and defines only `self.default_url_options`: Rails' `default_url_options`
//! is a class_attribute, so an instance's `_url` call reads the class's value, a subclass's instance too.
//! `EXPECTED` is what Rails 8.1.4 answers (`class_side_url_options_match_rails`).

use super::emit_and_run;

const LINKER: &str = r#"class Linker
  include Rails.application.routes.url_helpers

  def self.default_url_options
    { host: "class.example" }
  end

  def links
    [article_url(1), article_url(1, protocol: "https"), article_path(1)].join(" ")
  end
end

class SubLinker < Linker
end
"#;

const EXPECTED: &str = "http://class.example/articles/1 https://class.example/articles/1 /articles/1\nhttp://class.example/articles/1 https://class.example/articles/1 /articles/1";

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog().write("lib/linker.rb", LINKER)
}

#[test]
fn an_instance_url_reads_the_class_side_url_options() {
    app()
        .run_ruby(&format!(
            r##"got = [Linker.new.links, SubLinker.new.links].join("\n")
raise "urls: #{{got.inspect}}" unless got == {EXPECTED:?}
puts "class side url options passed"
"##
        ))
        .assert_passes();
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn an_instance_url_reads_the_class_side_url_options_on_spinel() {
    app()
        .run_spinel(&format!(
            r##"got = [Linker.new.links, SubLinker.new.links].join("\n")
raise "urls: #{{got.inspect}}" unless got == {EXPECTED:?}
puts "class side url options passed"
"##
        ))
        .assert_passes();
}

#[test]
fn class_side_url_options_match_rails() {
    let helper = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/rails_oracle.rb");
    let script = format!(
        r##"require {helper:?}
unless RailsOracle.available?("railties", "actionpack", "actionview")
  warn "skipping class side url options oracle: #{{RailsOracle.label}} needs railties, actionpack and actionview"
  exit 77
end
RailsOracle.activate("railties", "actionpack", "actionview")
require "rails"
require "action_controller/railtie"
class ClassSideUrlOracle < Rails::Application
  config.load_defaults 8.1
  config.eager_load = false
  config.hosts.clear
  config.logger = Logger.new(File::NULL)
  config.secret_key_base = "class-side-url-oracle"
  routes.append {{ resources :articles, only: [:show] }}
end
ClassSideUrlOracle.initialize!
{LINKER}print [Linker.new.links, SubLinker.new.links].join("\n")
"##
    );
    let output = std::process::Command::new("ruby").args(["-e", &script]).output().expect("Rails oracle");
    if output.status.code() == Some(77) {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr).trim());
        return;
    }
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout), EXPECTED);
}
