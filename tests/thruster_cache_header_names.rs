//! ThrusterCache reads `vary` and `cache-control` off every response the
//! app answers, by the lowercase name and then by the capitalized one
//! (ruby_overlay/runtime/thruster_cache.rb). The capitalized spelling of
//! those two names is written once: a response without the header is
//! answered as before, without rebuilding the name.

use std::path::Path;
use std::process::Command;

#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn a_response_header_is_found_under_either_spelling_without_rebuilding_the_name() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require "rack"
require File.expand_path("runtime/spinel/scaffold/ruby_overlay/runtime/thruster_cache", Dir.pwd)
# The lookup as it was written: the lowercase name, then each dash-separated part capitalized.
reference = ->(headers, name) { headers[name] || headers[name.split("-").map(&:capitalize).join("-")] }
names = ["vary", "cache-control", "etag", "content-type", "x-cache", "set-cookie", "www-authenticate", "x", "", "-", "a--b",
         "Vary", "CACHE-CONTROL", "cache_control", "vary ", "x-é"]
tables = [
  {}, { "vary" => "Accept" }, { "Vary" => "Accept" }, { "VARY" => "Accept" }, { "vary" => "a", "Vary" => "b" },
  { "cache-control" => "public, max-age=60" }, { "Cache-Control" => "public, max-age=60" }, { "Cache-control" => "x" },
  { "cache-control" => nil, "Cache-Control" => "kept" }, { "cache-control" => false, "Cache-Control" => "kept" },
  { "Etag" => "\"v1\"", "Content-Type" => "text/css", "X-Cache" => "hit", "Www-Authenticate" => "Basic" },
  { "X" => "1", "" => "empty", "-" => "dash", "A--B" => "2", "A-B" => "3", "X-É" => "4" },
]
count = 0
tables.each do |headers|
  names.each do |name|
    want = reference.(headers, name)
    got = ThrusterCache.header(headers, name)
    raise "FAILED: #{name.inspect} in #{headers.inspect} reads #{got.inspect}, was #{want.inspect}" unless got.equal?(want) || got == want
    raise "FAILED: #{name.inspect} in #{headers.inspect} reads another object" unless got.equal?(want)
    count += 1
  end
end
# A frozen name and a frozen table are read, not written.
frozen = { "Vary" => "Accept".freeze }.freeze
raise "FAILED: frozen" unless ThrusterCache.header(frozen, "vary".freeze).equal?(frozen["Vary"])
count += 1

# What every response is asked: a `vary` it does not have, a `cache-control`
# under either spelling. Nothing is allocated; rebuilding the name made six.
plain = { "content-type" => "text/html; charset=utf-8", "cache-control" => "max-age=0, private, must-revalidate" }
capitalized = { "Content-Type" => "text/html; charset=utf-8", "Cache-Control" => "public, max-age=60", "Vary" => "Accept" }
[[plain, "vary"], [plain, "cache-control"], [capitalized, "vary"], [capitalized, "cache-control"]].each do |headers, name|
  1000.times { ThrusterCache.header(headers, name) }
  before = GC.stat(:total_allocated_objects)
  1000.times { ThrusterCache.header(headers, name) }
  per_call = (GC.stat(:total_allocated_objects) - before).fdiv(1000)
  raise "FAILED: reading #{name.inspect} allocates #{per_call} objects per call, want 0" unless per_call < 0.5
  count += 1
end
# The rule the cache applies with them is unchanged.
{ [200, plain] => nil, [200, capitalized] => 60, [200, { "cache-control" => "public, max-age=60", "Vary" => "*" }] => nil,
  [200, { "Cache-Control" => "public, s-max-age=5, max-age=60" }] => 5, [404, capitalized] => nil, [200, {}] => nil }.each do |(status, headers), want|
  got = ThrusterCache.cache_lifetime(status, headers)
  raise "FAILED: lifetime of #{status} #{headers.inspect} is #{got.inspect}, want #{want.inspect}" unless got == want
  count += 1
end
puts "ALL OK #{count}"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 203"), "{stdout}");
}
