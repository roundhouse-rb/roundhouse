//! `link_to name, special_path` where an app helper defines `special_path`
//! links to what the helper answers: Rails mixes the app's helpers in
//! above the url helpers. `link_to`'s url classification took the name for
//! a route (`undefined method 'special_path' for module RouteHelpers`), and
//! a `_url` one for a route's absolute form. A route helper beside them
//! still links to its route.

use super::emit_and_run;

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .write("app/controllers/probes_controller.rb", r##"class ProbesController < ApplicationController
  def links
  end
end
"##)
        .write("app/helpers/probes_helper.rb", r##"module ProbesHelper
  def special_path
    "/special/place"
  end

  def device_url(id)
    "https://devices.test/#{id}"
  end
end
"##)
        .write("app/views/probes/links.html.erb", r##"<p id="r">[<%= link_to "S", special_path %>|<%= link_to "D", device_url(7) %>|<%= link_to "A", articles_path %>]</p>
"##)
        .write("config/routes.rb", r##"Rails.application.routes.draw do
  root "articles#index"

  resources :articles do
    resources :comments, only: [:create, :destroy]
  end

  get "probes/links", to: "probes#links"
end
"##)
        .write("test/controllers/probes_controller_test.rb", r##"require "test_helper"

class ProbesControllerTest < ActionDispatch::IntegrationTest
  test "link_to of an app helper's _path or _url links to what the helper answers" do
    get "/probes/links"
    assert_includes response.body, "[<a href=\"/special/place\">S</a>|<a href=\"https://devices.test/7\">D</a>|<a href=\"/articles\">A</a>]"
  end
end
"##)
}

#[test]
fn link_to_an_app_helpers_path_links_to_what_it_answers() {
    app().run_test("test/controllers/probes_controller_test.rb").assert_passes();
}

const SCRIPT: &str = r##"Db.exec("INSERT INTO articles (id, title, body, created_at, updated_at) VALUES (1, 'Title', 'Body', '2026-11-01 09:30:15', '2026-11-01 09:30:15')")
request = ActionDispatch::Request.new
request.request_method = "GET"
request.path = "/probes/links"
request.query_string = ""
ActionController::Current.request = request
controller = ProbesController.new
ActionController::Current.controller = controller
controller.request_method = "GET"
controller.params = {}
controller.process_action(:links)
body = controller.body
raise body unless body.include?("[<a href=\"/special/place\">S</a>|<a href=\"https://devices.test/7\">D</a>|<a href=\"/articles\">A</a>]")
puts "probe passed"
"##;

#[test]
#[ignore = "requires the Spinel toolchain"]
fn link_to_an_app_helpers_path_links_to_what_it_answers_on_spinel() {
    let run = app().run_spinel_with_db(SCRIPT);
    run.assert_passes();
    assert!(run.stdout.contains("probe passed"), "{}", run.stdout);
}
