//! A request constraint is asked with the request once its route's path matches, and a request it
//! refuses goes on to the routes after it, as Rails' router does (#727): a lambda, a guard object
//! and a Hash keyed by a request attribute, on CRuby and on Spinel.

use super::emit_and_run;

const ROUTES: &str = r#"  root "articles#index"
  get "/invite/:code", to: "gates#api", constraints: ->(req) { req.format == :json }
  get "/invite/:code", to: "gates#page"
  get "/door/:code", to: "gates#api", constraints: lambda { |req| req.format == :json }
  get "/door/:code", to: "gates#page"
  constraints subdomain: "admin" do
    get "/panel", to: "gates#admin"
  end
  get "/panel", to: "gates#public_panel"
  constraints(StaffOnly.new) do
    get "/desk", to: "gates#staff"
  end
  get "/desk", to: "gates#visitor"
"#;

const CONTROLLER: &str = r#"class GatesController < ApplicationController
  def api = render(plain: "api")
  def page = render(plain: "page")
  def admin = render(plain: "admin")
  def public_panel = render(plain: "public")
  def staff = render(plain: "staff")
  def visitor = render(plain: "visitor")
end
"#;

const STAFF_ONLY: &str = r#"class StaffOnly
  def matches?(request)
    request.headers["X-Staff"] == "1"
  end
end
"#;

const TEST: &str = r#"require "test_helper"

class GatesControllerTest < ActionDispatch::IntegrationTest
  test "a lambda on the format" do
    get "/invite/abc", headers: { "Accept" => "application/json" }
    assert_equal "api", response.body
    get "/invite/abc"
    assert_equal "page", response.body
  end

  test "a Kernel lambda on the format" do
    get "/door/abc", headers: { "Accept" => "application/json" }
    assert_equal "api", response.body
    get "/door/abc"
    assert_equal "page", response.body
  end

  test "a request-keyed Hash" do
    host! "admin.example.com"
    get "/panel"
    assert_equal "admin", response.body
    host! "www.example.com"
    get "/panel"
    assert_equal "public", response.body
  end

  test "a guard object" do
    get "/desk", headers: { "X-Staff" => "1" }
    assert_equal "staff", response.body
    get "/desk"
    assert_equal "visitor", response.body
  end
end
"#;

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .edit("config/routes.rb", "  root \"articles#index\"\n", ROUTES)
        .write("app/controllers/gates_controller.rb", CONTROLLER)
        .write("lib/staff_only.rb", STAFF_ONLY)
}

#[test]
fn constraints_are_asked_with_the_request_and_a_refused_one_reaches_the_next_route() {
    app()
        .write("test/controllers/gates_controller_test.rb", TEST)
        .run_test("test/controllers/gates_controller_test.rb")
        .assert_passes();
}

/// What the dispatcher needs before it chooses a route, through config.ru.
const DISPATCH: &str = r##"
require "rack"
require "rack/mock"
%w[nio websocket/driver].each { |feature| $LOADED_FEATURES << "#{feature}.rb" }
module NIO; end
module WebSocket; end
builder = Rack::Builder.new
builder.instance_eval(File.read("config.ru"), "config.ru")
client = Rack::MockRequest.new(builder.to_app)
check = ->(cond, what) { raise "FAILED: #{what}" unless cond }
from_dispatcher = -> { caller_locations(2, 1).first.path.end_with?("/main.rb") }
extensions = []
ActionDispatch::MimeNegotiation.singleton_class.prepend(Module.new {
  define_method(:format_name) { |extension, *rest| extensions << extension if from_dispatcher.call; super(extension, *rest) }
})
requests = 0
ActionDispatch::Request.singleton_class.prepend(Module.new {
  define_method(:new) { |*args, &blk| requests += 1 if from_dispatcher.call; super(*args, &blk) }
})
splits = []
String.prepend(Module.new {
  define_method(:split) { |*args, &blk|
    splits << self.dup if from_dispatcher.call
    super(*args, &blk)
  }
})
# The first route that fits the path, constraints not asked: what decides whether a constraint is asked at all.
first_fit = ->(path) { ActionDispatch::Router.match_from("GET", path, Main.route_table, 0) }
"##;

/// A request constraint is asked with a request the dispatcher builds for
/// it: an `ActionDispatch::Request` whose format comes from the path's
/// extension and the Accept header. A request whose first fitting route has
/// no constraint, or that fits no route, asks nothing: that request is not
/// built and the format is not read. The routes answer as before.
#[test]
fn a_route_with_no_constraint_asks_nothing_of_the_request() {
    let script = format!(
        "{DISPATCH}{}",
        r##"
["/articles/new", "/no-such-page", "/no-such-page.json", "/no.such/page.xml"].each do |path|
  fit = first_fit.(path)
  check.(fit.nil? || fit.guard < 0, "#{path} was meant to reach no constrained route")
  extensions.clear
  splits.clear
  requests = 0
  response = client.get(path)
  check.(response.status == (fit.nil? ? 404 : 200), "#{path} answers #{response.status}")
  check.(extensions.empty?, "#{path} reaches no constrained route and its format was read: #{extensions.inspect}")
  check.(requests == (fit.nil? ? 0 : 1), "#{path}: the dispatcher built #{requests} requests")
  check.(splits.none? { |text| text == path }, "#{path} was split by the dispatcher")
end
check.(client.get("/no-such-page.json").body.include?("\"status\":404"), "a .json 404 is still answered as JSON")
# A constrained route is asked, and a request it refuses goes on to the next route.
{ ["/invite/abc", {}] => "page", ["/invite/abc", { "HTTP_ACCEPT" => "application/json" }] => "api", ["/invite/abc.json", {}] => "api",
  ["/door/abc", {}] => "page", ["/door/abc.json", {}] => "api",
  ["/desk", {}] => "visitor", ["/desk", { "HTTP_X_STAFF" => "1" }] => "staff" }.each do |(path, env), body|
  check.(first_fit.(path).guard >= 0, "#{path} was meant to reach a constrained route first")
  extensions.clear
  requests = 0
  response = client.get(path, env)
  check.(response.body == body, "#{path} #{env.inspect} answers #{response.body.inspect}, want #{body.inspect}")
  check.(extensions.size == 1, "#{path}: the format was read #{extensions.size} times for its constraint")
  check.(requests == 2, "#{path}: the dispatcher built #{requests} requests, one for the constraint and one for the controller")
end
puts "lazy constraint request contract passed"
"##
    );
    app().run_ruby(&script).assert_passes();
}

/// The dispatcher reads the extension of the path's last segment for the
/// request a route constraint is asked with. A path with no dot has none
/// and is not split to look for one; a path with a dot is read as before,
/// whichever segment the dot is in.
#[test]
fn the_dispatcher_splits_a_path_for_its_extension_only_when_it_has_a_dot() {
    let script = format!(
        "{DISPATCH}{}",
        r##"
# The extension as the dispatcher has always read it: the last segment's text after its last dot.
reference = lambda do |path|
  segment = path.split("/").last.to_s
  segment.include?(".") ? segment.split(".").last.to_s : ""
end
constrained = 0
["/invite/abc", "/invite/abc/", "/invite/abc.json", "/invite/abc.JSON", "/invite/abc.", "/invite/.hidden", "/invite/a..b",
 "/invite/a.b.rss", "/door/abc", "/door/abc.xml", "/panel", "/panel.json", "/panel/", "/desk", "/desk.html"].each do |path|
  fit = first_fit.(path)
  next if fit.nil? || fit.guard < 0
  constrained += 1
  extensions.clear
  splits.clear
  client.get(path)
  check.(extensions.first == reference.(path), "#{path} is read with the extension #{extensions.first.inspect}, was #{reference.(path).inspect}")
  next if path.include?(".")
  check.(splits.none? { |text| text == path }, "#{path} has no dot and was split by the dispatcher")
end
check.(constrained >= 9, "only #{constrained} of the paths reached a constrained route")
puts "path extension contract passed"
"##
    );
    app().run_ruby(&script).assert_passes();
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn constraints_are_asked_on_spinel() {
    let script = r#"def answer(host, accept, path)
  request = ActionDispatch::Request.new
  request.host = host
  request.format = ActionDispatch::MimeNegotiation.format_name("", accept, "", "")
  matched = RouteTable.match_asking("GET", path, [RouteTable.root] + RouteTable.table, request)
  matched.nil? ? "none" : matched.action.to_s
end
raise "json" unless answer("www.example.com", "application/json", "/invite/abc") == "api"
raise "page" unless answer("www.example.com", "text/html,*/*", "/invite/abc") == "page"
raise "admin" unless answer("admin.example.com", "", "/panel") == "admin"
raise "public" unless answer("www.example.com", "", "/panel") == "public_panel"
raise "visitor" unless answer("www.example.com", "", "/desk") == "visitor"
puts "route guards passed"
"#;
    app().run_spinel(script).assert_passes();
}

/// A constraint the router asks has to type: `req.env["warden"]` is a gradual value, so calling
/// on it is an error at the constraint, not an unchecked send that raises once a request matches.
#[test]
fn a_constraint_that_does_not_type_is_an_error_where_it_is_written() {
    let (_, errors) = emit_and_run::real_blog()
        .edit(
            "config/routes.rb",
            "  root \"articles#index\"\n",
            "  root \"articles#index\"\n  get \"/desk\", to: \"articles#index\", constraints: ->(req) { req.env[\"warden\"].authenticate?(scope: :user) }\n",
        )
        .emit(roundhouse::project::BuildTarget::Ruby);
    assert!(errors.iter().any(|e| e.contains("request constraint") && e.contains("cannot type")), "{errors:#?}");
}
