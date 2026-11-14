//! The response headers handed to Rack without per-name work on every
//! response, on CRuby (ruby_overlay/runtime/response_header_names_cruby.rb):
//! the header store ends as the shared `commit_default_headers!` leaves it,
//! for every mix of headers the action set before; every header name gets
//! the Rack key `to_s.downcase` would give (value, encoding, frozen), before
//! and after it is remembered, past the cap, and for a name the caller
//! mutates afterwards; and the emitted app takes both paths.

use std::path::Path;
use std::process::Command;

#[path = "support/emit_and_run.rs"]
mod emit_and_run;
#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn emitted_ruby_writes_default_headers_in_one_pass() {
    emit_and_run::empty_app()
        .write("db/schema.rb", "ActiveRecord::Schema[8.1].define(version: 1) do\n create_table :widgets do |t|\n  t.string :name\n end\nend\n")
        .write("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n")
        .write(
            "app/controllers/widgets_controller.rb",
            "class WidgetsController < ApplicationController\n def show\n  headers[\"x-frame-options\"] = \"DENY\"\n  headers[\"X-Widget-Kind\"] = \"plain\"\n  render plain: 'ok'\n end\nend\n",
        )
        .write("config/routes.rb", "Rails.application.routes.draw do\n get '/ok', to: 'widgets#show'\nend\n")
        .run_ruby(r##"
raise "shared commit loaded" unless ActionController::Base.instance_method(:commit_default_headers!).owner == ActionController::DefaultHeadersOnePass
# The shared path stores `key.downcase`, a new String; the one-pass path a
# frozen constant: frozen match forms say the one-pass path wrote them.
probe = ActionController::Base.new
probe.commit_default_headers!
lower = probe.headers.instance_variable_get(:@lower)
raise "one-pass commit not taken: #{lower.inspect}" unless lower.size == 5 && lower.all?(&:frozen?)
status, headers, _body = Main.run_rack("REQUEST_METHOD" => "GET", "PATH_INFO" => "/ok", "QUERY_STRING" => "", "rack.input" => StringIO.new(""))
raise "status #{status}" unless status == 200
want = { "x-frame-options" => "DENY", "x-widget-kind" => "plain", "x-xss-protection" => "0",
         "x-content-type-options" => "nosniff", "x-permitted-cross-domain-policies" => "none",
         "referrer-policy" => "strict-origin-when-cross-origin" }
want.each { |k, v| raise "#{k}: #{headers[k].inspect} in #{headers.inspect}" unless headers[k] == v }
raise "a name is not downcased: #{headers.keys.inspect}" unless headers.keys.all? { |k| k == k.downcase }
raise "x-frame-options twice: #{headers.keys.inspect}" unless headers.keys.count { |k| k == "x-frame-options" } == 1
kept = ActionController.instance_variable_get(:@rh_rack_names)
raise "rack_response did not go through the name memo: #{kept.inspect}" unless kept && kept["X-Widget-Kind"].equal?(-"x-widget-kind")
puts "ALL OK"
"##)
        .assert_passes();
}

#[test]
fn one_pass_default_headers_match_the_shared_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"# encoding: utf-8
require_relative "runtime/ruby/action_controller/base"
require_relative "runtime/ruby/action_controller/default_headers"
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/header_validation_cruby"
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/response_headers_cruby"
B = ActionController::Base
S = ActionController::HeaderStore
shared_commit = B.instance_method(:commit_default_headers!)
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/response_header_names_cruby"
raise "commit not replaced" unless B.instance_method(:commit_default_headers!).owner == ActionController::DefaultHeadersOnePass
check = ->(what, cond) { raise "FAILED: #{what}" unless cond }
outcome = ->(&blk) { begin; [:ok, blk.call]; rescue => e; [e.class]; end }
str = ->(s) { s.is_a?(String) ? [s, s.encoding, s.frozen?] : s }

defaults = ["X-Frame-Options", "X-XSS-Protection", "X-Content-Type-Options", "X-Permitted-Cross-Domain-Policies", "Referrer-Policy"]
spell = [->(n) { n }, ->(n) { n.downcase }, ->(n) { n.upcase }]
others = [["Content-Type", "text/html"], ["Cache-Control", "private"], ["ETag", "W/\"1\""], ["Café", "v"],
          ["K-Kelvin", "k"], ["X-\xFF".b, "b"], ["X-Frame-Option", "near"], ["referrer-policyy", "near"]]
states = [[]]
32.times do |mask|
  spell.each_with_index do |sp, si|
    set = defaults.each_with_index.select { |_, i| mask[i] == 1 }.map { |n, i| [sp.(n), "app-#{i}-#{si}"] }
    states << set << (others.first(3) + set) << (set + others) << (others.rotate(mask % others.size) + set.reverse)
  end
end

# A controller whose store is in `@_headers`, as in the emitted trees, with an
# app-defined `headers` the one-pass path must not call; and one whose store
# is in `@headers` only, as in the unrenamed shared source, which takes the
# shared method. The shared method itself reads `@headers` here.
controller = lambda do |store, ivar|
  c = B.allocate
  c.instance_variable_set(ivar, store)
  c.define_singleton_method(:headers) { raise "the app's own headers" } if ivar == :@_headers
  c
end
surface = lambda do |store|
  probes = defaults + defaults.map(&:downcase) + others.map(&:first) + ["missing", ""]
  [store.size, (0...store.size).map { |i| [str.(store.key_at(i)), str.(store.val_at(i))] },
   store.instance_variable_get(:@lower).map { |l| [l, l.encoding] },
   store.slice(*probes).to_a, probes.map { |p| outcome.call { str.(store[p]) } }]
end
states.each do |state|
  [:@headers, :@_headers].each do |ivar|
    a = S.new
    b = S.new
    state.each { |k, v| a.store_value(k, v); b.store_value(k, v) }
    check.("return #{state.inspect}", outcome.call { shared_commit.bind_call(controller.(a, :@headers)) } ==
                                      outcome.call { controller.(b, ivar).commit_default_headers! })
    check.("store #{ivar} #{state.inspect}", surface.(a) == surface.(b))
    # What comes after must read the appended entries as it reads the shared ones.
    [a, b].each { |s| s["x-frame-options"] = "after"; s.delete("Referrer-Policy"); s.store_value("REFERRER-POLICY", "y"); s["X-New"] = "n" }
    check.("after #{ivar} #{state.inspect}", surface.(a) == surface.(b))
  end
end

# A store of another class, a subclass included, takes the shared method,
# errors included.
class SubStore < S
  def store_value(k, v) = super(k, "sub:#{v}")
end
[-> { {} }, -> { { "X-Frame-Options" => "DENY" } }, -> { SubStore.new }, -> { nil }, -> { 42 }].each do |make|
  ha = make.call
  hb = make.call
  ra = outcome.call { shared_commit.bind_call(controller.(ha, :@headers)) }
  # Both ivars hold it: the one-pass path reads `@_headers`, the shared
  # method it hands over to reads `@headers` in the unrenamed source.
  rb = outcome.call { controller.(hb, :@headers).tap { |c| c.instance_variable_set(:@_headers, hb) }.commit_default_headers! }
  check.("other store #{ha.inspect}", ra == rb)
  check.("other store contents #{ha.inspect}", ha.is_a?(S) ? surface.(ha) == surface.(hb) : ha == hb)
end
# Rack names: the key `headers[k.to_s.downcase] = v` would store.
class SubString < String; end
names = ["Content-Type", "X-Frame-Options", "x-request-id", "ETag", "Set-Cookie", "Café", "CAFÉ", "K-Kelvin",
         "İnfo", "X-\xFF".b, "X-\xFF".dup.force_encoding("UTF-8"), "Content-Type".encode("US-ASCII"),
         "Content-Type".encode("UTF-16LE"), "Content-Type".b, "", "Content-Type".freeze, :Sym, :"X-A", 42, nil,
         SubString.new("X-Sub"),
         # Full downcase differs from case folding (ß), and non-ASCII names on the fallback path.
         "Straße", :"CAFÉ", :"Straße"]
key_of = ->(k) { h = {}; h[k] = 1; str.(h.keys.first) }
3.times do
  names.each do |n|
    want = outcome.call { n.to_s.downcase }
    got = outcome.call { ActionController.rack_header_name(n) }
    check.("rack name #{n.inspect}", want.size == got.size && want[0] == got[0])
    check.("rack key #{n.inspect}", key_of.(want[1]) == key_of.(got[1])) if want[0] == :ok
  end
end
# A hit answers the kept, interned name itself.
hit = ActionController.rack_header_name("X-Hit")
check.("memo hit", hit.frozen? && hit.equal?(-"x-hit") && ActionController.rack_header_name(+"X-Hit").equal?(hit))
table = ActionController.instance_variable_get(:@rh_rack_names)
ActionController.rack_header_name("X-Hit")
check.("a hit leaves the table alone", ActionController.instance_variable_get(:@rh_rack_names).equal?(table))
# A hit is served from the table, by the name asked for.
planted = { "X-Planted" => -"planted" }.freeze
ActionController.instance_variable_set(:@rh_rack_names, planted)
check.("hit served from the memo", ActionController.rack_header_name(+"X-Planted").equal?(-"planted"))
check.("a hit keeps the memo", ActionController.instance_variable_get(:@rh_rack_names).equal?(planted))
ActionController.instance_variable_set(:@rh_rack_names, table)
mutable = +"X-Mutable"
check.("first", ActionController.rack_header_name(mutable) == "x-mutable")
mutable << "-Changed"
check.("mutated", ActionController.rack_header_name(mutable) == "x-mutable-changed")
check.("original after mutation", ActionController.rack_header_name("X-Mutable") == "x-mutable")
300.times { |i| check.("cap #{i}", key_of.(ActionController.rack_header_name("X-Many-#{i}")) == key_of.("x-many-#{i}")) }
kept = ActionController.instance_variable_get(:@rh_rack_names)
check.("cap kept #{kept.size}", kept.size == ActionController::RACK_NAMES_CAP && kept.frozen?)
check.("remembered keys and names frozen", kept.keys.all?(&:frozen?) && kept.values.all?(&:frozen?))
check.("UTF-8 only", kept.keys.all? { |k| k.encoding == Encoding::UTF_8 })
puts "ALL OK #{states.size} stores"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("ALL OK"));
}
