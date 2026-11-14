//! `safe_join` answers a safe string, as Rails' SafeBuffer, so an `h(...)`
//! of what it joined leaves the markup alone. The runtime joined the parts
//! into a plain String, which `h` escaped again: `&lt;b&gt;x&lt;/b&gt; y`.
//! CRuby only: Spinel's runtime has no safe-string type (as for `raw`).

use super::emit_and_run;

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .write("app/controllers/probes_controller.rb", r##"class ProbesController < ApplicationController
  def joined
  end
end
"##)
        .write("app/helpers/probes_helper.rb", r##"module ProbesHelper
  def joined
    safe_join([tag.b("x"), "y"], " ")
  end

  def rejoined
    h(joined)
  end
end
"##)
        .write("app/views/probes/joined.html.erb", r##"<p id="r">[<%= rejoined %>]</p>
"##)
        .write("config/routes.rb", r##"Rails.application.routes.draw do
  root "articles#index"

  resources :articles do
    resources :comments, only: [:create, :destroy]
  end

  get "probes/joined", to: "probes#joined"
end
"##)
        .write("test/controllers/probes_controller_test.rb", r##"require "test_helper"

class ProbesControllerTest < ActionDispatch::IntegrationTest
  test "safe_join answers a safe string, which h leaves alone" do
    get "/probes/joined"
    assert_includes response.body, "[<b>x</b> y]"
  end
end
"##)
}

#[test]
fn safe_join_answers_a_safe_string() {
    app().run_test("test/controllers/probes_controller_test.rb").assert_passes();
}
