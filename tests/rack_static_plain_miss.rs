//! Rack::Static passes a plain path it cannot serve straight to the app on
//! CRuby (ruby_overlay/runtime/rack_static_cruby.rb): for every path and
//! every way Rack::Static is mounted, the answer (status, headers, body),
//! what the app receives and the exceptions are those of Rack::Static's own
//! `call`; and the emitted tree loads the shortcut.

use std::path::Path;
use std::process::Command;

#[path = "support/emit_and_run.rs"]
mod emit_and_run;
#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn emitted_ruby_boot_loads_the_rack_static_shortcut() {
    emit_and_run::empty_app()
        .write("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n")
        .run_ruby(r##"
require "rack/mock_request"
raise "shortcut not loaded" unless defined?(RackStaticPlainMiss) && Rack::Static.ancestors.first == RackStaticPlainMiss
inner = ->(env) { [200, { "x-app" => "1" }, ["app #{env["PATH_INFO"]}"]] }
static = Rack::Static.new(inner, urls: ["/assets"], root: "static")
status, headers, body = static.call(Rack::MockRequest.env_for("/rooms/1"))
raise "not passed to the app: #{status} #{headers}" unless status == 200 && headers["x-app"] == "1" && body.to_a == ["app /rooms/1"]
puts "ALL OK"
"##)
        .assert_passes();
}

#[test]
fn plain_misses_take_rack_statics_own_branch() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"# encoding: utf-8
require "rack"
require "rack/static"
require "rack/mock_request"
require "tmpdir"
require "fileutils"
original = Rack::Static.instance_method(:call)
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/rack_static_cruby"
raise "call not replaced" unless Rack::Static.instance_method(:call).owner == RackStaticPlainMiss
check = ->(what, cond) { raise "FAILED: #{what}" unless cond }
outcome = lambda do |&blk|
  status, headers, body = blk.call
  parts = []
  body.each { |b| parts << b }
  body.close if body.respond_to?(:close)
  [:ok, status, headers.to_h, parts.join]
rescue => e
  [e.class, e.message]
end
# Counted, and switchable to the older racks that ask `can_serve` about the
# raw path (before 2.2.13, 3.0.14, 3.1.12): both on Rack::Utils.
$cleaned = 0
$raw = false
Rack::Utils.singleton_class.prepend(Module.new do
  def clean_path_info(p)
    $cleaned += 1
    $raw ? p : super
  end

  def unescape_path(s)
    $raw ? s : super
  end
end)

Dir.mktmpdir do |dir|
  FileUtils.mkdir_p(File.join(dir, "assets", "sub"))
  File.write(File.join(dir, "assets", "app.css"), "css")
  File.write(File.join(dir, "assets", "app.css.gz"), "gz")
  File.write(File.join(dir, "icon.png"), "png")
  File.write(File.join(dir, "index.html"), "index")
  File.write(File.join(dir, "favicon.ico"), "ico")
  configs = [
    { urls: ["/assets", "/icon.png", "/icon.svg"], root: dir, header_rules: [[:all, { "cache-control" => "public" }]] },
    { urls: ["/assets/", "/icon.png"], root: dir },
    { urls: { "/" => "/index.html", "/home" => "/index.html" }, root: dir },
    { urls: [""], root: dir, index: "index.html" },
    { urls: ["/assets"], root: dir, cascade: true },
    { urls: ["/assets"], root: dir, gzip: true },
    { urls: ["assets"], root: dir },
    { root: dir }
  ]
  paths = ["", "/", "/assets", "/assets/", "/assets/app.css", "/assets/app.css/", "/assetsx", "/assets/missing.css",
           "/assets/sub/", "/icon.png", "/icon.png/", "/icon.svg", "/rooms/1", "/rooms/1/", "/up", "rooms", "assets/app.css",
           "//assets/app.css", "/x/../assets/app.css", "/./assets/app.css", "/assets/./app.css", "../assets/app.css",
           "./assets", "/assets/..", "/%61ssets/app.css", "/assets%2Fapp.css", "/rooms%2F1", "/é", "/\xFF".b,
           "/assets\\app.css", "/home", "/home/", "/favicon.ico", "/index.html", "/rooms/1".b, "/rooms/1".encode("US-ASCII"),
           "/\xFF".b.force_encoding("UTF-8"), "/x\\..\\assets\\app.css", "/home\\", nil, 42]
  inner = lambda do |env|
    env["x.calls"] = env.fetch("x.calls", 0) + 1
    [200, { "x-app" => "1" }, ["app #{env["PATH_INFO"].inspect}"]]
  end
  matrix = lambda do |mode|
    configs.each do |opts|
      paths.each do |p|
        [nil, "gzip"].each do |enc|
          envs = 2.times.map do
            env = Rack::MockRequest.env_for("/")
            env["PATH_INFO"] = p
            env["HTTP_ACCEPT_ENCODING"] = enc if enc
            env
          end
          want = outcome.call { original.bind_call(Rack::Static.new(inner, opts), envs[0]) }
          got = outcome.call { Rack::Static.new(inner, opts).call(envs[1]) }
          plain = envs.map { |e| e.reject { |k, _| k == "rack.input" || k == "rack.errors" } }
          check.("#{mode} #{opts.inspect} #{p.inspect} #{enc}:\n shared #{want.inspect} #{plain[0]["x.calls"]}\n fast #{got.inspect} #{plain[1]["x.calls"]}",
                 want == got && plain[0] == plain[1])
        end
      end
    end
  end
  matrix.(:current)
  # The racks that ask about the raw path.
  $raw = true
  matrix.(:raw_path)
  $raw = false
  # A platform whose File::ALT_SEPARATOR is a backslash (Windows, JRuby there).
  seps = Rack::Utils::PATH_SEPS
  begin
    Rack::Utils.send(:remove_const, :PATH_SEPS)
    Rack::Utils.const_set(:PATH_SEPS, Regexp.union("/", "\\"))
    matrix.(:backslash)
  ensure
    Rack::Utils.send(:remove_const, :PATH_SEPS)
    Rack::Utils.const_set(:PATH_SEPS, seps)
  end
  # The shortcut is taken: a plain page path is never cleaned; an escaped one is.
  static = Rack::Static.new(inner, configs[0])
  ["/rooms/1", "/up", "/rooms/1/", ""].each do |p|
    $cleaned = 0
    static.call(Rack::MockRequest.env_for("/").merge("PATH_INFO" => p))
    check.("#{p.inspect} cleaned #{$cleaned} times", $cleaned == 0)
  end
  $cleaned = 0
  static.call(Rack::MockRequest.env_for("/").merge("PATH_INFO" => "/%72ooms"))
  check.("an escaped path is cleaned", $cleaned == 1)
end
puts "ALL OK"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("ALL OK"));
}
