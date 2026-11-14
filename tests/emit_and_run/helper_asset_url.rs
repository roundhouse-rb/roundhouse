//! A helper's bare `asset_url` reaches the runtime's, as a view's reaches
//! ActionView's (`undefined method 'asset_url' for module ProbesHelper`
//! before), and an app helper's own `asset_url` still wins. On the Ruby
//! family, `asset_url` and `image_url` answer the asset's path on the
//! request's host, as Rails does with no `asset_host`, an absolute path
//! included; a full URL and a `data:` source pass as given.
//! With `skip_pipeline: true` a relative source is under its public directory: `/raw.png`, and `/images/raw.png` for an image. A blank source answers blank.

use super::emit_and_run;

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .write("app/assets/images/icon.png", r##"icon
"##)
        .write("app/controllers/probes_controller.rb", r##"class ProbesController < ApplicationController
  def icon
  end
end
"##)
        .write("app/helpers/probes_helper.rb", r##"module ProbesHelper
  def icon_urls
    [asset_url("icon.png"), image_url("icon.png"), asset_url("/x.png"), asset_url("data:x"), asset_url("https://cdn.test/y.png"), asset_url("raw.png", skip_pipeline: true), image_url("raw.png", skip_pipeline: true)].join(" ")
  end

  def blank_urls
    "{" + asset_url("") + "|" + image_url("", skip_pipeline: true) + "}"
  end
end
"##)
        .write("app/views/probes/icon.html.erb", r##"<p id="r">[<%= icon_urls %>]</p>
<p id="b"><%= blank_urls %></p>
"##)
        .write("config/routes.rb", r##"Rails.application.routes.draw do
  root "articles#index"

  resources :articles do
    resources :comments, only: [:create, :destroy]
  end

  get "probes/icon", to: "probes#icon"
end
"##)
        .write("test/controllers/probes_controller_test.rb", r##"require "test_helper"

class ProbesControllerTest < ActionDispatch::IntegrationTest
  test "a helper's asset_url and image_url answer the asset's URL on the request's host" do
    get "/probes/icon"
    assert_match %r{\[(http://www.example.com/assets/icon-[0-9a-f]+\.png) \1 http://www.example.com/x.png data:x https://cdn.test/y.png http://www.example.com/raw.png http://www.example.com/images/raw.png\]}, response.body
    assert_includes response.body, "<p id=\"b\">{|}</p>"
  end
end
"##)
}

#[test]
fn a_helpers_asset_url_is_the_assets_url_on_the_request_host() {
    app().run_test("test/controllers/probes_controller_test.rb").assert_passes();
}

const SCRIPT: &str = r##"Db.exec("INSERT INTO articles (id, title, body, created_at, updated_at) VALUES (1, 'Title', 'Body', '2026-11-01 09:30:15', '2026-11-01 09:30:15')")
request = ActionDispatch::Request.new
request.request_method = "GET"
request.path = "/probes/icon"
request.query_string = ""
ActionController::Current.request = request
controller = ProbesController.new
ActionController::Current.controller = controller
controller.request_method = "GET"
controller.params = {}
controller.process_action(:icon)
body = controller.body
urls = body[/\[([^\]]*)\]/, 1].to_s.split(" ")
raise body unless urls.length == 7 && urls[0].start_with?("http") && urls[0].include?("/assets/icon-") && urls[1] == urls[0]
raise body unless urls[2].start_with?("http") && urls[2].end_with?("/x.png") && urls[3] == "data:x" && urls[4] == "https://cdn.test/y.png"
raise body unless urls[5].start_with?("http") && urls[5].end_with?("/raw.png") && !urls[5].include?("/images/") && urls[6].start_with?("http") && urls[6].end_with?("/images/raw.png")
raise body unless body.include?("<p id=\"b\">{|}</p>")
puts "probe passed"
"##;

#[test]
#[ignore = "requires the Spinel toolchain"]
fn a_helpers_asset_url_is_the_assets_url_on_the_request_host_on_spinel() {
    let run = app().run_spinel_with_db(SCRIPT);
    run.assert_passes();
    assert!(run.stdout.contains("probe passed"), "{}", run.stdout);
}

/// An app helper's own `asset_url` is what a helper's bare call reaches:
/// Rails mixes the app's helpers in above ActionView's.
#[test]
fn an_app_helpers_own_asset_url_wins() {
    app()
        .write(
            "app/helpers/application_helper.rb",
            "module ApplicationHelper\n  def asset_url(source, skip_pipeline: false)\n    \"/own/\" + source\n  end\nend\n",
        )
        .edit(
            "test/controllers/probes_controller_test.rb",
            r#"assert_match %r{\[(http://www.example.com/assets/icon-[0-9a-f]+\.png) \1 http://www.example.com/x.png data:x https://cdn.test/y.png http://www.example.com/raw.png http://www.example.com/images/raw.png\]}, response.body"#,
            r#"assert_match %r{\[/own/icon.png http://www.example.com/assets/icon-[0-9a-f]+\.png /own//x.png /own/data:x /own/https://cdn.test/y.png /own/raw.png http://www.example.com/images/raw.png\]}, response.body"#,
        )
        .edit(
            "test/controllers/probes_controller_test.rb",
            r#"<p id=\"b\">{|}</p>"#,
            r#"<p id=\"b\">{/own/|}</p>"#,
        )
        .run_test("test/controllers/probes_controller_test.rb")
        .assert_passes();
}
