//! A view's `**opts` on a block builder's call (`m.item("a", "/a", **@opts)`
//! inside `menu do |m|`) stays a keyword splat, as Ruby passes it. The
//! keyword lowering makes the splat positional and marks it
//! (`ERASED_KEYWORD_SPLAT`) for the Ruby emit to write back; the view's
//! ivar-to-local and helper rewrites rebuilt the argument without the mark,
//! so `item(label, url, **opts)` got a third positional argument: `wrong
//! number of arguments (given 3, expected 2)`. A populated and an empty
//! Hash.

use super::emit_and_run;

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .write("app/controllers/probes_controller.rb", r##"class ProbesController < ApplicationController
  def menu
    @opts = { tone: "loud" }
    @none = {}
  end
end
"##)
        .write("app/helpers/probes_helper.rb", r##"module ProbesHelper
  def menu
    builder = MenuBuilder.new
    yield builder
    builder.items.join(",")
  end
end
"##)
        .write("app/models/menu_builder.rb", r##"class MenuBuilder
  attr_reader :items

  def initialize
    @items = []
  end

  def item(label, url, **opts)
    @items << "#{label}@#{url}#{opts[:tone]}"
    nil
  end
end
"##)
        .write("app/views/probes/menu.html.erb", r##"<p id="r">[<%= menu do |m| %><% m.item("a", "/a", **@opts) %><% m.item("b", "/b", **@none) %><% end %>]</p>
"##)
        .write("config/routes.rb", r##"Rails.application.routes.draw do
  root "articles#index"

  resources :articles do
    resources :comments, only: [:create, :destroy]
  end

  get "probes/menu", to: "probes#menu"
end
"##)
        .write("test/controllers/probes_controller_test.rb", r##"require "test_helper"

class ProbesControllerTest < ActionDispatch::IntegrationTest
  test "a view's **opts on a block builder's call stays a keyword splat" do
    get "/probes/menu"
    assert_includes response.body, "[a@/aloud,b@/b]"
  end
end
"##)
}

#[test]
fn a_views_keyword_splat_on_a_builder_call_stays_one() {
    app().run_test("test/controllers/probes_controller_test.rb").assert_passes();
}

const SCRIPT: &str = r##"Db.exec("INSERT INTO articles (id, title, body, created_at, updated_at) VALUES (1, 'Title', 'Body', '2026-11-01 09:30:15', '2026-11-01 09:30:15')")
request = ActionDispatch::Request.new
request.request_method = "GET"
request.path = "/probes/menu"
request.query_string = ""
ActionController::Current.request = request
controller = ProbesController.new
ActionController::Current.controller = controller
controller.request_method = "GET"
controller.params = {}
controller.process_action(:menu)
body = controller.body
raise body unless body.include?("[a@/aloud,b@/b]")
puts "probe passed"
"##;

#[test]
#[ignore = "requires the Spinel toolchain"]
fn a_views_keyword_splat_on_a_builder_call_stays_one_on_spinel() {
    let run = app().run_spinel_with_db(SCRIPT);
    run.assert_passes();
    assert!(run.stdout.contains("probe passed"), "{}", run.stdout);
}
