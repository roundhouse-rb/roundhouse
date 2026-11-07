//! A routing method a gem mixes into the mapper, as doorkeeper's `use_doorkeeper` is: its body draws
//! the routes, here from a block that configures it, and they answer where the call stands.

use super::emit_and_run;

const PING_ROUTES: &str = r##"module PingRoutes
  module Helper
    def use_pings(options = {}, &block)
      PingRoutes::Drawer.new(self, &block).draw(options[:path] || "pings")
    end
  end

  class Drawer
    def initialize(routes, &block)
      @routes = routes
      @controller = "pings"
      instance_eval(&block) if block
    end

    def controller(name)
      @controller = name
    end

    def draw(path)
      @routes.scope path, as: "pings" do
        @routes.get "/", controller: @controller, action: :show, as: :show
      end
    end
  end

  def self.install!
    ActionDispatch::Routing::Mapper.include PingRoutes::Helper
  end
end
"##;

#[test]
fn a_gems_routing_method_draws_routes_that_answer() {
    emit_and_run::real_blog()
        .write("lib/ping_routes.rb", PING_ROUTES)
        .write(
            "app/controllers/pongs_controller.rb",
            "class PongsController < ApplicationController\n  def show\n    render plain: \"pong #{pings_show_path}\"\n  end\nend\n",
        )
        .edit(
            "config/routes.rb",
            "  root \"articles#index\"\n",
            "  root \"articles#index\"\n  use_pings path: \"ping\" do\n    controller \"pongs\"\n  end\n",
        )
        .write(
            "test/controllers/pongs_controller_test.rb",
            "require \"test_helper\"\n\nclass PongsControllerTest < ActionDispatch::IntegrationTest\n  test \"the drawn route answers\" do\n    get \"/ping\"\n    assert_response :success\n    assert_equal \"pong /ping\", response.body\n  end\nend\n",
        )
        .run_test("test/controllers/pongs_controller_test.rb")
        .assert_passes();
}

const BELL_ROUTES: &str = r##"module Bells
  mattr_accessor :prefix
  @@prefix = "bells"

  def self.setup
    yield self
  end

  module Helper
    def use_bells(*names)
      options = names.extract_options!
      names.map! { |name| name.to_sym }
      base = { as: "bells" }.merge(options.slice(:as))
      with_options controller: "bells" do |routes|
        names.each do |name|
          routes.get "#{Bells.prefix}/#{name}", action: name, as: "#{base[:as]}_#{name}"
        end
      end
    end
  end

  def self.install!
    ActionDispatch::Routing::Mapper.include Bells::Helper
  end
end
"##;

/// The method takes its options as devise's `devise_for` does
/// (`extract_options!`, `map!`, a merged and sliced Hash), draws through
/// `with_options`, and reads a setting an initializer gave through the
/// gem's `setup { |config| … }`.
#[test]
fn a_gems_routing_method_reads_its_options_and_its_setup_block() {
    emit_and_run::real_blog()
        .write("lib/bells.rb", BELL_ROUTES)
        .write("config/initializers/bells.rb", "Bells.setup do |config|\n  config.prefix = \"chimes\"\nend\n")
        .write(
            "app/controllers/bells_controller.rb",
            "class BellsController < ApplicationController\n  def ding\n    render plain: \"ding #{bell_ding_path} #{bell_dong_path}\"\n  end\n\n  def dong\n    render plain: \"dong\"\n  end\nend\n",
        )
        .edit("config/routes.rb", "  root \"articles#index\"\n", "  root \"articles#index\"\n  use_bells \"ding\", :dong, as: \"bell\"\n")
        .write(
            "test/controllers/bells_controller_test.rb",
            "require \"test_helper\"\n\nclass BellsControllerTest < ActionDispatch::IntegrationTest\n  test \"the drawn routes answer\" do\n    get \"/chimes/ding\"\n    assert_response :success\n    assert_equal \"ding /chimes/ding /chimes/dong\", response.body\n  end\nend\n",
        )
        .run_test("test/controllers/bells_controller_test.rb")
        .assert_passes();
}
