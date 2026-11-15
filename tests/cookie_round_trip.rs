//! The CRuby overlay's cookie round trip
//! (ruby_overlay/runtime/cookie_round_trip_cruby.rb) answers what the shared
//! runtime answers: a remembered Cookie header parses to an equal Hash of
//! fresh Strings (same encodings, nothing shared with an earlier answer),
//! and a permanent cookie's Expires is the shared formula at every instant,
//! remembered only within the second it was computed in.

use std::path::Path;
use std::process::Command;

#[path = "support/emit_and_run.rs"]
mod emit_and_run;
#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn emitted_ruby_boot_loads_the_cookie_round_trip() {
    emit_and_run::empty_app()
        .write("db/schema.rb", "ActiveRecord::Schema[8.1].define(version: 1) do\n create_table :widgets do |t|\n  t.string :name\n end\nend\n")
        .write("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n")
        .write("app/controllers/widgets_controller.rb", "class WidgetsController < ApplicationController\n def show\n  render plain: 'ok'\n end\nend\n")
        .write("config/routes.rb", "Rails.application.routes.draw do\n get '/ok', to: 'widgets#show'\nend\n")
        .run_ruby(r#"
raise "shared parse_cookies loaded" unless CgiIo.method(:parse_cookies).owner == CgiIoCookieMemo
raise "shared permanent_expires loaded" unless ActionController::CookieJar.method(:permanent_expires).owner == CookieJarPermanentExpiresMemo
raise "parse" unless CgiIo.parse_cookies("a=1; b=%41") == { a: "1", b: "A" }
raise "expires" unless ActionController::CookieJar.permanent_expires.end_with?(" GMT")
puts "ALL OK"
"#).assert_passes();
}

#[test]
fn remembered_cookie_round_trip_matches_the_shared_runtime() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
# What ruby_overlay/boot.rb has loaded before cgi_io.
require "cgi"
require_relative "runtime/spinel/cgi_io"
require_relative "runtime/ruby/action_controller/cookies"
shared_parse = CgiIo.method(:parse_cookies)
shared_expires = ActionController::CookieJar.method(:permanent_expires)
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/cookie_round_trip_cruby"
raise "parse not replaced" if CgiIo.method(:parse_cookies).owner == shared_parse.owner

check = ->(what, cond) { raise "FAILED: #{what}" unless cond }
deep = ->(h) { h.map { |k, v| [k, v, v.encoding, v.frozen?] } }
headers = [nil, "", "a=1", "a=1; b=2", " a = 1 ;b=2 ", "a=%41%2B+x", "a=%zz", "noeq; a=1", "=1; a=2", "a=1; a=2",
           "s=" + "x" * 4096, "u=caf\xC3\xA9", "b=\xFF".b, "b=\xFF".force_encoding("UTF-8"), "e=", "a=1;;b=2",
           "session_token=eyJfcmFpbHMiOnsibWVzc2FnZSI6IkltRSJ9fQ%3D%3D--0123456789abcdef; _campfire_session=abc"]
outcome = ->(&blk) { begin; blk.call; rescue => e; e.class; end }
# A header the shared parser refuses (split on invalid UTF-8 raises) is
# refused the same way, every time, and never remembered.
raising = headers.select { |h| outcome.call { shared_parse.call(h) }.is_a?(Class) }
check.("one refused header in the corpus", raising.size == 1)
raising.each do |h|
  2.times { check.("refused #{h.inspect}", outcome.call { CgiIo.parse_cookies(h) } == outcome.call { shared_parse.call(h) }) }
end
headers -= raising
headers.each do |h|
  want = shared_parse.call(h)
  first = CgiIo.parse_cookies(h)
  second = CgiIo.parse_cookies(h)
  check.("miss #{h.inspect}", deep.(first) == deep.(want))
  check.("hit #{h.inspect}", deep.(second) == deep.(want))
  check.("fresh hash #{h.inspect}", !first.equal?(second))
  first.each { |k, v| check.("fresh value #{h.inspect} #{k}", !v.equal?(second[k])) }
  unless second.empty?
    k = second.keys.first
    second[k] << "MUTATED" unless second[k].frozen?
    second[:added] = "x"
    check.("a mutated answer leaks #{h.inspect}", deep.(CgiIo.parse_cookies(h)) == deep.(want))
  end
end
# Interleaved headers, and four threads.
check.("interleaved", 3.times.all? { headers.all? { |h| deep.(CgiIo.parse_cookies(h)) == deep.(shared_parse.call(h)) } })
threads = 4.times.map do |t|
  Thread.new do
    rng = Random.new(t)
    500.times.all? { h = headers[rng.rand(headers.size)]; deep.(CgiIo.parse_cookies(h)) == deep.(shared_parse.call(h)) }
  end
end
check.("threads", threads.all?(&:value))

# Expires against the shared formula at chosen instants: Time.now is stubbed
# as travel_to stubs it, with a queue so a computation can straddle a tick.
queue = []
Time.singleton_class.alias_method(:__real_now, :now)
Time.singleton_class.define_method(:now) { queue.empty? ? __real_now : queue.shift }
at = ->(*args) { Time.utc(*args) }
instants = [at.(2024, 2, 29, 12, 0, 0.25), at.(2024, 2, 29, 12, 0, 0.75), at.(2026, 12, 31, 23, 59, 59.5),
            at.(2027, 1, 1, 0, 0, 0.0), at.(2000, 1, 1, 0, 0, 0.0)]
instants.each do |t|
  queue.replace([t, t]); want = shared_expires.call
  queue.replace([t, t, t]); got = ActionController::CookieJar.permanent_expires
  check.("expires at #{t}", got == want && got.encoding == want.encoding && !got.frozen?)
  queue.replace([t]); again = ActionController::CookieJar.permanent_expires
  check.("expires remembered at #{t}", again == want && !again.equal?(got) && !again.frozen?)
end
# A computation that straddles a tick is answered but not kept.
s = at.(2030, 6, 1, 10, 0, 0.999); s2 = at.(2030, 6, 1, 10, 0, 1.001)
queue.replace([s, s2, s2]); straddled = ActionController::CookieJar.permanent_expires
queue.replace([s, s, s]); same_second = ActionController::CookieJar.permanent_expires
queue.replace([s]); check.("the straddled text was not kept for the earlier second", same_second == shared_expires.call)
queue.replace([s2]); want2 = shared_expires.call
queue.replace([s2, s2, s2]); later = ActionController::CookieJar.permanent_expires
check.("straddled text is the one computed", straddled == want2)
check.("next second recomputed", later == want2)
queue.replace([s, s, s]); back = ActionController::CookieJar.permanent_expires
queue.replace([s]); want_s = shared_expires.call
check.("a clock stepped back gets its own second", back == want_s)
puts "ALL OK"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("ALL OK"));
}
