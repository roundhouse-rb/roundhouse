//! `<% page_title "Orders" %>`: a template's statement tag calls an app
//! helper for its effect, as Rails' `<% %>` does, and discards its value.
//! The view walker took a bare helper call in statement position for a
//! shape it doesn't lower, and dropped it with a warning: the helper never
//! ran, so neither its `content_for` deposit nor the layout's `<title>`
//! from it appeared.

use super::emit_and_run;

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .write("app/controllers/probes_controller.rb", r##"class ProbesController < ApplicationController
  def statement
  end
end
"##)
        .write("app/helpers/probes_helper.rb", r##"module ProbesHelper
  def put_slot(value)
    content_for(:probe_slot, value)
  end

  def put_block_slot
    content_for(:probe_block) { "blk" }
  end

  def put_title(value)
    content_for(:title, value)
  end

  def read_slot
    content_for(:probe_slot)
  end

  def read_missing
    content_for(:probe_missing)
  end
end
"##)
        .write("app/views/probes/statement.html.erb", r##"<% put_slot "val" %>
<% put_title "From a statement" %>
<p id="r">[<%= read_slot %>]</p>
"##)
        .write("config/routes.rb", r##"Rails.application.routes.draw do
  root "articles#index"

  resources :articles do
    resources :comments, only: [:create, :destroy]
  end

  get "probes/statement", to: "probes#statement"
end
"##)
        .write("test/controllers/probes_controller_test.rb", r##"require "test_helper"

class ProbesControllerTest < ActionDispatch::IntegrationTest
  test "a template's statement tag calls an app helper for its effect" do
    get "/probes/statement"
    assert_includes response.body, "[val]"
    assert_includes response.body, "<title>From a statement</title>"
  end
end
"##)
}

#[test]
fn a_statement_tag_calls_an_app_helper_for_its_effect() {
    app().run_test("test/controllers/probes_controller_test.rb").assert_passes();
}

const SCRIPT: &str = r##"Db.exec("INSERT INTO articles (id, title, body, created_at, updated_at) VALUES (1, 'Title', 'Body', '2026-11-01 09:30:15', '2026-11-01 09:30:15')")
request = ActionDispatch::Request.new
request.request_method = "GET"
request.path = "/probes/statement"
request.query_string = ""
ActionController::Current.request = request
controller = ProbesController.new
ActionController::Current.controller = controller
controller.request_method = "GET"
controller.params = {}
controller.process_action(:statement)
body = controller.body
raise body unless body.include?("[val]") && body.include?("<title>From a statement</title>")
puts "probe passed"
"##;

#[test]
#[ignore = "requires the Spinel toolchain"]
fn a_statement_tag_calls_an_app_helper_for_its_effect_on_spinel() {
    let run = app().run_spinel_with_db(SCRIPT);
    run.assert_passes();
    assert!(run.stdout.contains("probe passed"), "{}", run.stdout);
}
