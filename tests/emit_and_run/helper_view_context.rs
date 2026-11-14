//! An object a helper builds with `self` (`NavItem.new(self, path)`) is
//! handed the view, and reads `request` and `controller` through it, as
//! Rails' view context answers them. Emitted, `self` is the helper module,
//! which had neither: `undefined method 'request' for module ProbesHelper`.
//! `request.path_info` is the runtime Request's too.

use super::emit_and_run;

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .write("app/controllers/probes_controller.rb", r##"class ProbesController < ApplicationController
  def nav
  end
end
"##)
        .write("app/helpers/probes_helper.rb", r##"module ProbesHelper
  def nav_item(path)
    NavItem.new(self, path)
  end
end
"##)
        .write("app/models/nav_item.rb", r##"class NavItem
  def initialize(view, path)
    @view = view
    @path = path
  end

  def active?
    @view.request.path_info == @path
  end

  def controller_name
    @view.controller.controller_name
  end
end
"##)
        .write("app/views/probes/nav.html.erb", r##"<p id="r">[<%= nav_item("/probes/nav").active? %>|<%= nav_item("/x").active? %>|<%= nav_item("/").controller_name %>]</p>
"##)
        .write("config/routes.rb", r##"Rails.application.routes.draw do
  root "articles#index"

  resources :articles do
    resources :comments, only: [:create, :destroy]
  end

  get "probes/nav", to: "probes#nav"
end
"##)
        .write("test/controllers/probes_controller_test.rb", r##"require "test_helper"

class ProbesControllerTest < ActionDispatch::IntegrationTest
  test "an object a helper builds with self reads the request and controller through it" do
    get "/probes/nav"
    assert_includes response.body, "[true|false|probes]"
  end
end
"##)
}

#[test]
fn an_object_built_with_a_helpers_self_reads_the_request() {
    app().run_test("test/controllers/probes_controller_test.rb").assert_passes();
}

const SCRIPT: &str = r##"Db.exec("INSERT INTO articles (id, title, body, created_at, updated_at) VALUES (1, 'Title', 'Body', '2026-11-01 09:30:15', '2026-11-01 09:30:15')")
request = ActionDispatch::Request.new
request.request_method = "GET"
request.path = "/probes/nav"
request.query_string = ""
ActionController::Current.request = request
controller = ProbesController.new
ActionController::Current.controller = controller
controller.request_method = "GET"
controller.params = {}
controller.process_action(:nav)
body = controller.body
raise body unless body.include?("[true|false|probes]")
puts "probe passed"
"##;

#[test]
#[ignore = "requires the Spinel toolchain"]
fn an_object_built_with_a_helpers_self_reads_the_request_on_spinel() {
    let run = app().run_spinel_with_db(SCRIPT);
    run.assert_passes();
    assert!(run.stdout.contains("probe passed"), "{}", run.stdout);
}
