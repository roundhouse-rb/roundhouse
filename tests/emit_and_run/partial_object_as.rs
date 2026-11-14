//! `render partial: "shared/badge", object: record, as: "someone"` hands
//! the partial its object as the local `someone`, a String `as:` as well as
//! a Symbol, as Rails' partial renderer does (`as.to_sym`). Without `as:`
//! the local is the partial's own name (`shared/_person` reads `person`).
//! The partial took the object under its directory's name (`shared`) and
//! read `someone` bare: `wrong number of arguments (given 0, expected
//! 1..3)`, the partial's own module function. An `object:` with `locals:` beside it is still the `as:` local, and a partial rendered `as: :class` takes `locals: { class: }` under that name.

use super::emit_and_run;

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .write("app/controllers/probes_controller.rb", r##"class ProbesController < ApplicationController
  def person
    @article = Article.first
  end
end
"##)
        .write("app/views/probes/person.html.erb", r##"<p id="r">[<%= render partial: "shared/badge", object: @article, as: "someone" %>|<%= render partial: "shared/badge", object: @article, as: :someone %>|<%= render partial: "shared/person", object: @article %>|<%= render partial: "shared/badge", locals: { someone: @article } %>|<%= render partial: "shared/card", collection: [@article], as: :article %>|<%= render partial: "shared/card", object: @article, locals: { article: @article } %>|<%= render partial: "shared/badge", object: @article, as: :someone, locals: { label: "x" } %>|<%= render partial: "shared/klass", collection: [@article], as: :class %>|<%= render partial: "shared/klass", locals: { class: @article } %>]</p>
"##)
        .write("app/views/shared/_badge.html.erb", r##"<b><%= someone.title %></b>
"##)
        .write("app/views/shared/_card.html.erb", r##"<i><%= article.title %></i>
"##)
        .write("app/views/shared/_klass.html.erb", r##"<u><%= local_assigns[:class].title %></u>
"##)
        .write("app/views/shared/_person.html.erb", r##"<span><%= person.title %></span>
"##)
        .write("config/routes.rb", r##"Rails.application.routes.draw do
  root "articles#index"

  resources :articles do
    resources :comments, only: [:create, :destroy]
  end

  get "probes/person", to: "probes#person"
end
"##)
        .write("test/controllers/probes_controller_test.rb", r##"require "test_helper"

class ProbesControllerTest < ActionDispatch::IntegrationTest
  test "an object render's as: names the partial's local, a String as well as a Symbol" do
    get "/probes/person"
    t = Article.first.title
    assert_includes response.body, "[<b>#{t}</b>\n|<b>#{t}</b>\n|<span>#{t}</span>\n|<b>#{t}</b>\n|<i>#{t}</i>\n|<i>#{t}</i>\n|<b>#{t}</b>\n|<u>#{t}</u>\n|<u>#{t}</u>\n]"
  end
end
"##)
}

#[test]
fn an_object_renders_as_names_the_partials_local() {
    app().run_test("test/controllers/probes_controller_test.rb").assert_passes();
}

const SCRIPT: &str = r##"Db.exec("INSERT INTO articles (id, title, body, created_at, updated_at) VALUES (1, 'Title', 'Body', '2026-11-01 09:30:15', '2026-11-01 09:30:15')")
request = ActionDispatch::Request.new
request.request_method = "GET"
request.path = "/probes/person"
request.query_string = ""
ActionController::Current.request = request
controller = ProbesController.new
ActionController::Current.controller = controller
controller.request_method = "GET"
controller.params = {}
controller.process_action(:person)
body = controller.body
raise body unless body.include?("[<b>Title</b>\n|<b>Title</b>\n|<span>Title</span>\n|<b>Title</b>\n|<i>Title</i>\n|<i>Title</i>\n|<b>Title</b>\n|<u>Title</u>\n|<u>Title</u>\n]")
puts "probe passed"
"##;

#[test]
#[ignore = "requires the Spinel toolchain"]
fn an_object_renders_as_names_the_partials_local_on_spinel() {
    let run = app().run_spinel_with_db(SCRIPT);
    run.assert_passes();
    assert!(run.stdout.contains("probe passed"), "{}", run.stdout);
}
