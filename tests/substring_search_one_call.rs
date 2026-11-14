//! `ActionController.find_substr` and `find_last` find the host in every
//! URL a redirect is checked against and in every absolute URL a `_url`
//! helper builds. On CRuby
//! (ruby_overlay/runtime/substring_search_cruby.rb) two ASCII-only Strings
//! are searched with String#index and #rindex: the answer is the shared
//! walk's for every pair, in any encoding, and the search allocates
//! nothing where the walk cut a substring per position.

use std::path::Path;
use std::process::Command;

#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn substring_searches_match_the_shared_walk_and_allocate_nothing_for_ascii() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require_relative "runtime/ruby/action_controller/base"
AC = ActionController
first = AC.method(:find_substr)
last = AC.method(:find_last)
host = AC.method(:location_host)
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/substring_search_cruby"
outcome = ->(&blk) { begin; blk.call; rescue => e; e.class; end }

hays = ["", "a", "://", "http://example.com/a/b?c#d", "127.0.0.1:4411", "user:pw@host:80", "a@b@c", "[::1]:3000", "aaa", "abab",
        "x://y://z", "/", "//", "a/b/c", ":", "::", "host", "HOST:80", " a ", "a\u00e9b", "\u00e9", "\u00e9:\u00e9", "a:\xff".b, "\xff:\xff".b,
        "\xc3".dup.force_encoding("UTF-8"), "a:b".encode("US-ASCII"), "a:b".encode("UTF-16LE"), "a:b".b, "a:\u3042".encode("Shift_JIS"),
        "frozen:hay".freeze, "x" * 200 + ":" + "y" * 200]
needles = ["", "://", "/", "?", "#", "@", ":", "a", "ab", "aa", "b", "\u00e9", "\xff".b, ":\xff".b, "\u00e9:".encode("ISO-8859-1"),
           ":".encode("UTF-16LE"), ":".encode("US-ASCII"), "::", "host", "zzzz" * 60, "@".freeze]
count = 0
hays.each do |hay|
  needles.each do |needle|
    [[first, :find_substr], [last, :find_last]].each do |walk, name|
      want = outcome.call { walk.call(hay, needle) }
      got = outcome.call { AC.public_send(name, hay, needle) }
      raise "FAILED: #{name}(#{hay.inspect} #{hay.encoding}, #{needle.inspect} #{needle.encoding}) answers #{got.inspect}, the walk #{want.inspect}" unless got == want
      count += 1
    end
  end
end
# What is not a String goes to the walk as it is.
[[nil, ":"], ["a:b", nil], [:sym, ":"], ["a:b", :":"], [42, "4"]].each do |hay, needle|
  [[first, :find_substr], [last, :find_last]].each do |walk, name|
    want = outcome.call { walk.call(hay, needle) }
    got = outcome.call { AC.public_send(name, hay, needle) }
    raise "FAILED: #{name}(#{hay.inspect}, #{needle.inspect}) answers #{got.inspect}, the walk #{want.inspect}" unless got == want
    count += 1
  end
end

# The search itself: nothing allocated for the Strings a request has.
[["127.0.0.1:4411", "://", :find_substr], ["http://127.0.0.1:4411/articles/1", "://", :find_substr], ["127.0.0.1:4411/articles/1", "/", :find_substr],
 ["127.0.0.1:4411", ":", :find_last], ["user@example.com", "@", :find_last], ["example.com", "@", :find_last]].each do |hay, needle, name|
  1000.times { AC.public_send(name, hay, needle) }
  before = GC.stat(:total_allocated_objects)
  1000.times { AC.public_send(name, hay, needle) }
  per_call = (GC.stat(:total_allocated_objects) - before).fdiv(1000)
  raise "FAILED: #{name}(#{hay.inspect}, #{needle.inspect}) allocates #{per_call} objects per call, want 0" unless per_call < 0.5
  count += 1
end

# What is built on them answers as before: the host a redirect is checked
# against, the port of a host, an absolute URL.
urls = ["http://example.com/a", "https://user:pw@Example.COM:8443/a?b#c", "//cdn.example.com/x", "///path", "/articles/1", "", "javascript:alert(1)",
        "http://127.0.0.1:4411/articles/1", "http://h\u00e9.example/a", "ftp://a@b@c/d", "http://example.com?x=/y", "http://example.com#/y"]
urls.each do |url|
  raise "FAILED: location_host(#{url.inspect}) answers #{AC.location_host(url).inspect}, was #{host.call(url).inspect}" unless AC.location_host(url) == host.call(url)
  count += 1
end
{ "127.0.0.1:4411" => "4411", "example.com" => "", "[::1]:3000" => "3000", "[::1]" => "", "example.com:" => "", "example.com:80a" => "", "a:b:8080" => "8080" }.each do |hostport, port|
  raise "FAILED: url_port_of(#{hostport.inspect}) answers #{AC.url_port_of(hostport).inspect}, want #{port.inspect}" unless AC.url_port_of(hostport) == port
  count += 1
end
{ ["http://", "127.0.0.1:4411", "4411", "/articles/1"] => "http://127.0.0.1:4411/articles/1",
  ["http://", "example.com:80", "80", "/a"] => "http://example.com/a",
  ["", "https://example.com", "", "/a"] => "https://example.com/a",
  ["https", "example.com:443", "443", "/a"] => "https://example.com/a" }.each do |args, url|
  raise "FAILED: build_host_url(#{args.inspect}) answers #{AC.build_host_url(*args).inspect}, want #{url.inspect}" unless AC.build_host_url(*args) == url
  count += 1
end
puts "ALL OK #{count}"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 1341"), "{stdout}");
}
