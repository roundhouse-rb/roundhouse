//! `Router.decode_capture` decodes every GET's path (the public-file lookup)
//! and every route capture. On CRuby
//! (ruby_overlay/runtime/router_decode_cruby.rb) a value with nothing to
//! decode, ASCII without a `%`, is copied in one step: the answer is the
//! shared decoder's for every input (value, encoding, a new unfrozen
//! String), with the same exception for the inputs it refuses.

use std::path::Path;
use std::process::Command;

#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn decode_capture_matches_the_shared_decoder_and_copies_a_plain_value_once() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require_relative "runtime/ruby/action_dispatch/router"
R = ActionDispatch::Router
shared = R.method(:decode_capture)
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/router_decode_cruby"
outcome = ->(&blk) { begin; v = blk.call; [:ok, v, v.encoding, v.frozen?, v.valid_encoding?]; rescue => e; [e.class, e.message]; end }

corpus = ["", "/", "/up", "/rooms/1", "/rooms/1/messages", "/users/me/sidebar", "a b", "a+b", "%", "%2", "%41",
          "%2541", "%zz", "a%20b+c", "a%2fb%3F%23", "100%25", "bad%zz%2", "jos%C3%A9", "%C3", "%E2%82", "%ED%A0%80",
          "%F4%90%80%80", "%C0%AF", "/assets/app-1a2b.css", "x" * 300]
256.times { |b| corpus << b.chr.b << "a#{b.chr}".b; corpus << b.chr.dup.force_encoding("UTF-8") if b < 128 }
alphabet = ["a", "/", "+", "%", "2", "5", "F", "A", "9", "C", "3", "\xFF".b, "é"]
alphabet.each { |x| alphabet.each { |y| corpus << (x.b + y.b) << (x + y rescue (x.b + y.b)) } }
corpus << "/rooms/1".encode("US-ASCII") << "/rooms/1".encode("UTF-16LE") << "/rooms/1".freeze << "/up".b.freeze
corpus << "é".encode("ISO-8859-1") << "/a".encode("Shift_JIS")
bad = 0
corpus.each do |value|
  want = outcome.call { shared.call(value) }
  got = outcome.call { R.decode_capture(value) }
  raise "FAILED: decode_capture(#{value.inspect} #{value.encoding}): shared #{want.inspect}, copied #{got.inspect}" unless want == got
  bad += 1 if want[0] != :ok
  if want[0] == :ok
    copy = R.decode_capture(value)
    raise "FAILED: #{value.inspect} is answered as the input object" if copy.equal?(value)
    bytes = value.bytes
    copy << "!"
    raise "FAILED: #{value.inspect} changed when its answer did" unless value.bytes == bytes
  end
end
raise "FAILED: no refused input in the corpus" if bad.zero?
[nil, 42, :sym].each do |value|
  raise "FAILED: #{value.inspect}" unless outcome.call { shared.call(value) }[0] == outcome.call { R.decode_capture(value) }[0]
end

# The copy: a plain path costs the same few objects whatever its length;
# the shared walk makes two per byte.
["/articles/1", "/rooms/1/messages", "/users/me/sidebar", "x" * 300].each do |path|
  1000.times { R.decode_capture(path) }
  before = GC.stat(:total_allocated_objects)
  1000.times { R.decode_capture(path) }
  per_call = (GC.stat(:total_allocated_objects) - before).fdiv(1000)
  raise "FAILED: a plain #{path.bytesize}-byte path allocates #{per_call} objects per call, want at most 3" unless per_call <= 3.5
end
# The route match that decodes: a capture comes back decoded, a plain one as it is.
table = [ActionDispatch::Router::Route.new("GET", "/articles/:id", :articles, :show)]
[["/articles/7", "7"], ["/articles/a%20b", "a b"], ["/articles/jos%C3%A9", "josé"]].each do |path, id|
  hit = R.match("GET", path, table)
  raise "FAILED: #{path} matched #{hit && hit.path_params.inspect}" unless hit && hit.path_params["id"] == id && hit.path_params["id"].encoding == Encoding::UTF_8
end
puts "ALL OK #{corpus.size} inputs, #{bad} refused"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 1009 inputs, 312 refused"), "{stdout}");
}
