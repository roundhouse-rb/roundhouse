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

/// The dispatcher hands the header store itself to whatever writes the
/// response, where it handed a Hash filled from the store entry by entry.
/// `each` yields what that Hash yields: every name as first written with
/// its value, in the order written; so the Rack headers and the CGI lines
/// written from the store are the ones written from the copy.
#[test]
fn the_header_store_yields_the_pairs_of_its_copy_into_a_hash() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require "stringio"
require_relative "runtime/ruby/action_controller/base"
require_relative "runtime/ruby/action_controller/default_headers"
require_relative "runtime/spinel/cgi_io"
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/header_validation_cruby"
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/response_headers_cruby"
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/response_header_names_cruby"
S = ActionController::HeaderStore
# What the dispatcher handed on before: a Hash filled from the store, entry by entry.
copy = lambda do |store|
  out = {}
  i = 0
  while i < store.size
    out[store.key_at(i)] = store.val_at(i)
    i += 1
  end
  out
end
# The two readers of what the dispatcher hands on: Main.rack_response and CgiIo.write_response.
rack = lambda do |pairs|
  headers = {}
  pairs.each { |k, v| headers[ActionController.rack_header_name(k)] = v unless v.nil? }
  headers
end
cgi = lambda do |pairs|
  io = StringIO.new
  CgiIo.write_response(io, 200, "", extra_headers: pairs)
  io.string
end
writes = [
  [],
  [["Content-Disposition", "inline"]],
  [["X-A", "1"], ["X-B", "2"], ["X-A", "3"]],
  [["x-case", "lower"], ["X-Case", "upper"]],
  [["Cache-Control", "no-store"], ["ETag", "\"v\""], ["Vary", "Accept"], ["X-Empty", ""]],
  [["Bad Name", "x"], ["X-Ok", "y"], ["X-Bad-Value", "a\nb"]],
  [["X-Num", "7"], ["Referrer-Policy", "no-referrer"]],
]
count = 0
writes.each do |pairs|
  [false, true].each do |defaults|
    [false, true].each do |deleted|
      store = S.new
      pairs.each { |k, v| store[k] = v }
      if defaults
        store.rh_append("X-Frame-Options", "x-frame-options", "SAMEORIGIN") if store.index_of("X-Frame-Options") < 0
        store.rh_append("Referrer-Policy", "referrer-policy", "strict-origin-when-cross-origin") if store.index_of("Referrer-Policy") < 0
      end
      store.delete(pairs.first.first) if deleted && !pairs.empty?
      want = copy.(store)
      got = []
      back = store.each { |k, v| got << [k, v] }
      raise "FAILED: #{pairs.inspect} yields #{got.inspect}, the copy holds #{want.to_a.inspect}" unless got == want.to_a
      raise "FAILED: each answers #{back.class}, want the store" unless back.equal?(store)
      raise "FAILED: #{pairs.inspect} gives Rack #{rack.(store).to_a.inspect}, was #{rack.(want).to_a.inspect}" unless rack.(store).to_a == rack.(want).to_a
      raise "FAILED: #{pairs.inspect} writes other CGI lines" unless cgi.(store) == cgi.(want)
      count += 1
    end
  end
end
# Yielding the pairs allocates nothing; the copy was a Hash.
store = S.new
[["Content-Disposition", "inline"], ["X-A", "1"], ["X-B", "2"]].each { |k, v| store[k] = v }
sink = 0
200.times { store.each { |k, v| sink += v.bytesize } }
before = GC.stat(:total_allocated_objects)
1000.times { store.each { |k, v| sink += v.bytesize } }
per_call = (GC.stat(:total_allocated_objects) - before).fdiv(1000)
raise "FAILED: each allocates #{per_call} objects per call, want 0" unless per_call < 0.5
count += 1
puts "ALL OK #{count}"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 29"), "{stdout}");
}
