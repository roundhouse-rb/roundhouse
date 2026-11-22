//! Request header names mapped to Rack env keys once per name on CRuby
//! (ruby_overlay/runtime/header_names_cruby.rb, and ThrusterCache's
//! rack_header_key): every read answers what the shared mapping answers,
//! before and after the name is remembered, past the cap, and for a name the
//! caller mutates afterwards.

use std::path::Path;
use std::process::Command;

#[path = "support/emit_and_run.rs"]
mod emit_and_run;
#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn emitted_ruby_boot_loads_the_header_name_memo() {
    emit_and_run::empty_app()
        .write("db/schema.rb", "ActiveRecord::Schema[8.1].define(version: 1) do\n create_table :widgets do |t|\n  t.string :name\n end\nend\n")
        .write("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n")
        .write("app/controllers/widgets_controller.rb", "class WidgetsController < ApplicationController\n def show\n  render plain: 'ok'\n end\nend\n")
        .write("config/routes.rb", "Rails.application.routes.draw do\n get '/ok', to: 'widgets#show'\nend\n")
        .run_ruby(r#"
raise "shared env_name loaded" unless ActionDispatch::Http::Headers.method(:env_name).owner == ActionDispatch::Http::HeadersEnvNameMemo
raise "env_name" unless ActionDispatch::Http::Headers.new({ "HTTP_ACCEPT" => "x" })["Accept"] == "x"
puts "ALL OK"
"#).assert_passes();
}

#[test]
fn remembered_header_names_match_the_shared_mapping() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require "rack"
require_relative "runtime/ruby/action_dispatch/headers"
H = ActionDispatch::Http::Headers
shared = H.method(:env_name)
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/header_names_cruby"
require File.expand_path("runtime/spinel/scaffold/ruby_overlay/runtime/thruster_cache", Dir.pwd)
raise "env_name not replaced" if H.method(:env_name).owner == shared.owner
check = ->(what, cond) { raise "FAILED: #{what}" unless cond }
outcome = ->(&blk) { begin; [:ok, blk.call]; rescue => e; [e.class, e.message]; end }

env = { "HTTP_ACCEPT_ENCODING" => "gzip", "CONTENT_TYPE" => "text/html", "HTTPS" => "on", "rack.input" => "io",
        "HTTP_X_A_B" => "xab", "HTTP_ACCEPT" => "*/*", "HTTP_IF_NONE_MATCH" => "\"e\"" }
names = ["Accept-Encoding", "accept-encoding", "ACCEPT-ENCODING", "HTTP_ACCEPT", "Content-Type", "content_length",
         "HTTPS", "https", "rack.input", "X-A_b", "X-A-B", "x-a-b", "", "Ünï", "If-None-Match", "Turbo-Frame",
         "Accept-Encoding".b, "Accept-Encoding".encode("US-ASCII"), "Accept-Encoding".encode("UTF-16LE"),
         "Accept-Encoding".freeze, :accept, :"Accept-Encoding", nil, 42,
         # Answers that keep the key's own encoding: a name returned as given,
         # a CGI variable, an env key already.
         "rack.input".b, "Content-Type".b, "HTTP_ACCEPT".b, "content_length".b, "HTTPS".encode("US-ASCII")]
3.times do
  names.each do |n|
    want = outcome.call { shared.call(n) }
    got = outcome.call { H.env_name(n) }
    check.("env_name(#{n.inspect})", want == got && (want[0] != :ok || !want[1].is_a?(String) || want[1].encoding == got[1].encoding))
    h = H.new(env)
    %i[[] key? include?].each do |m|
      check.("#{m}(#{n.inspect})", outcome.call { h.public_send(m, n) } == outcome.call { env.public_send(m == :[] ? :[] : :key?, shared.call(n)) })
    end
    check.("fetch(#{n.inspect})", outcome.call { h.fetch(n) } == outcome.call { env.key?(shared.call(n)) ? env[shared.call(n)] : raise(KeyError, "key not found: #{n}") })
    check.("fetch default(#{n.inspect})", outcome.call { h.fetch(n, :d) } == outcome.call { env.fetch(shared.call(n), :d) })
  end
end
# A name the caller mutates after asking does not change what is kept:
# asked while there is room, so the name is kept when it is mutated.
m = +"Accept"
check.("before mutation", H.env_name(m) == "HTTP_ACCEPT")
check.("Accept is kept", H.instance_variable_get(:@rh_env_names).key?("Accept"))
m << "-Language"
check.("mutated name", H.env_name(m) == shared.call(m))
check.("kept name intact", H.env_name("Accept") == "HTTP_ACCEPT" && H.instance_variable_get(:@rh_env_names)["Accept"] == "HTTP_ACCEPT")
# Past the cap: names keep answering right whether or not they are kept.
300.times { |i| n = "X-Name-#{i}"; check.("cap #{n}", H.env_name(n) == shared.call(n)) }
check.("cap held", H.instance_variable_get(:@rh_env_names).size <= 256)
check.("kept answers frozen", H.instance_variable_get(:@rh_env_names).all? { |k, v| k.frozen? && v.frozen? })

# ThrusterCache's per-name Rack key against the formula it replaced.
formula = ->(name) { "HTTP_#{name.upcase.tr("-", "_")}" }
2.times do
  ["upgrade", "connection", "range", "Accept", "accept-encoding", "If-None-Match", "x_y", "Ünï", "range".b, :accept].each do |n|
    want = outcome.call { formula.call(n) }
    got = outcome.call { ThrusterCache.rack_header_key(n) }
    check.("rack_header_key(#{n.inspect})", want == got && (want[0] != :ok || want[1].encoding == got[1].encoding))
  end
end
300.times { |i| n = "x-h-#{i}"; check.("rack cap #{n}", ThrusterCache.rack_header_key(n) == formula.call(n)) }
check.("rack cap held", ThrusterCache.instance_variable_get(:@rack_keys).size <= 256)
puts "ALL OK"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("ALL OK"));
}
