//! A helper's `content_for` deposits into and reads the view's slots, as a
//! template's own does: Rails mixes the helper into the view whose
//! `content_for` it calls. A value and a block deposit (the layout's
//! `<title>` reads one), `content_for(:slot)` reads, and a slot nothing
//! deposited reads nil. A nil value reads the slot, as Rails' `content_for` does when given no content. The emitted helper module called
//! `content_for` bare: `undefined method 'content_for' for module
//! ProbesHelper`.

use super::emit_and_run;

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .write("app/controllers/probes_controller.rb", r##"class ProbesController < ApplicationController
  def deposit
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

  def read_nil
    content_for(:probe_slot, nil)
  end

  def read_missing
    content_for(:probe_missing)
  end
end
"##)
        .write("app/views/probes/deposit.html.erb", r##"<%= put_slot "val" %><%= put_block_slot %><%= put_title "From a helper" %>
<p id="r">[<%= read_slot %>|<%= content_for(:probe_block) %>|<%= read_missing.nil? %>|<%= read_nil %>|<%= content_for(:probe_slot, nil) %>]</p>
"##)
        .write("config/routes.rb", r##"Rails.application.routes.draw do
  root "articles#index"

  resources :articles do
    resources :comments, only: [:create, :destroy]
  end

  get "probes/deposit", to: "probes#deposit"
end
"##)
        .write("test/controllers/probes_controller_test.rb", r##"require "test_helper"

class ProbesControllerTest < ActionDispatch::IntegrationTest
  test "a helper's content_for deposits into and reads the view's slots" do
    get "/probes/deposit"
    assert_includes response.body, "[val|blk|true|val|val]"
    assert_includes response.body, "<title>From a helper</title>"
  end
end
"##)
}

#[test]
fn a_helpers_content_for_deposits_into_the_views_slots() {
    app().run_test("test/controllers/probes_controller_test.rb").assert_passes();
}

const SCRIPT: &str = r##"Db.exec("INSERT INTO articles (id, title, body, created_at, updated_at) VALUES (1, 'Title', 'Body', '2026-11-01 09:30:15', '2026-11-01 09:30:15')")
request = ActionDispatch::Request.new
request.request_method = "GET"
request.path = "/probes/deposit"
request.query_string = ""
ActionController::Current.request = request
controller = ProbesController.new
ActionController::Current.controller = controller
controller.request_method = "GET"
controller.params = {}
controller.process_action(:deposit)
body = controller.body
raise body unless body.include?("[val|blk|true|val|val]") && body.include?("<title>From a helper</title>")
puts "probe passed"
"##;

#[test]
#[ignore = "requires the Spinel toolchain"]
fn a_helpers_content_for_deposits_into_the_views_slots_on_spinel() {
    let run = app().run_spinel_with_db(SCRIPT);
    run.assert_passes();
    assert!(run.stdout.contains("probe passed"), "{}", run.stdout);
}
