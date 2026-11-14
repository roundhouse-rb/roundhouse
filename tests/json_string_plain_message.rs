//! `MessageVerifier.json_string` writes the JSON string inside every signed
//! value (a session, a signed cookie, a signed id, a blob key). On CRuby
//! (ruby_overlay/runtime/message_verifier_cruby.rb) a message with nothing
//! to escape is copied between two quotes in one step, and the answer is
//! the shared byte walk's for every message: the same bytes, tagged UTF-8,
//! in a new unfrozen String.

use std::path::Path;
use std::process::Command;

#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn json_string_matches_the_shared_walk_and_copies_a_plain_message_once() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require_relative "runtime/spinel/base64"
require_relative "runtime/spinel/message_digest_cruby"
require_relative "runtime/ruby/action_controller/message_verifier"
MV = ActionController::MessageVerifier
walk = MV.method(:json_string)
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/message_verifier_cruby"

same = lambda do |value|
  want = walk.call(value)
  got = MV.json_string(value)
  raise "FAILED: #{value.inspect} gives #{got.inspect}, the walk #{want.inspect}" unless got.bytes == want.bytes
  raise "FAILED: #{value.inspect} is tagged #{got.encoding}, the walk #{want.encoding}" unless got.encoding == want.encoding
  raise "FAILED: #{value.inspect} validity differs" unless got.valid_encoding? == want.valid_encoding?
  raise "FAILED: #{value.inspect} comes back frozen" if got.frozen?
  raise "FAILED: #{value.inspect} comes back as the message itself" if got.equal?(value)
end

# Every byte, alone and inside a plain message, as text and as bytes.
count = 0
256.times do |b|
  one = b.chr
  [one, "ab" + one + "cd", one + "x", "x" + one, one * 3].each do |s|
    same.(s)
    same.(s.dup.force_encoding("UTF-8"))
    same.(s.dup.force_encoding("US-ASCII"))
    count += 3
  end
end
# Messages of the shapes that are signed: a session's url-encoded pairs, a
# global id, a blob key with its disposition, text that needs every escape.
session = "_csrf_token=%22#{"A" * 43}%22&user_id=42"
[session, "", "gid://app/User/42", "abc123|inline; filename=\"a b.png\"", "café   \u{1F600}",
 "<script>alert(\"x\") && 1</script>", "line\nbreak\ttab\rback\\slash", "nul\u0000byte", "\u007f", "a" * 5000,
 ("a" * 4999) + "\"", "\"" + ("a" * 4999)].each { |s| same.(s); count += 1 }
# Other encodings: only the bytes count.
["\x82\xa0".dup.force_encoding("Shift_JIS"), "ab".encode("UTF-16LE"), "\xff\xfe".b, "\xc3".dup.force_encoding("UTF-8"),
 "plain".encode("ISO-8859-1"), "caf\xe9".dup.force_encoding("ISO-8859-1")].each { |s| same.(s); count += 1 }
# What is not a String is written through its `to_s`, as the walk does.
[42, nil, :sym, 1.5].each { |v| same.(v); count += 1 }
# A frozen message is not touched and not answered itself.
frozen = "frozen-message".freeze
same.(frozen); count += 1

# The copy: whatever the length, one object for an ASCII-only message with
# no rewritten byte (it is searched as it is), two for another (searched
# through a binary copy), and a few per rewritten byte (three on Ruby 3.4,
# one on 4.0) over six when some are; the walk makes one per byte.
["_csrf_token=%22#{"A" * 43}%22", "a" * 500, "café " * 40, session, ("<" * 20) + ("a" * 200), ("<" * 20) + ("é" * 100)].each do |s|
  rewritten = s.b.scan(/[\x00-\x1f"&<>\\]/n).size
  most = (s.ascii_only? ? 0 : 1) + (rewritten.zero? ? 1 : 6 + 3 * rewritten)
  1000.times { MV.json_string(s) }
  before = GC.stat(:total_allocated_objects)
  1000.times { MV.json_string(s) }
  per_call = (GC.stat(:total_allocated_objects) - before).fdiv(1000)
  raise "FAILED: a #{s.bytesize}-byte message with #{rewritten} rewritten bytes allocates #{per_call} objects per call, want at most #{most}" unless per_call <= most + 0.5
end
# The escapes are the walk's own, byte for byte: nothing is spelled here twice.
MV::JSON_STRING_ESCAPES.each do |byte, escape|
  raise "FAILED: #{byte.inspect} is written #{escape.inspect}, the walk #{walk.call(byte)[1..-2].inspect}" unless walk.call(byte).b == "\"".b + escape + "\""
end
raise "FAILED: #{MV::JSON_STRING_ESCAPES.size} bytes are rewritten, the walk rewrites 37" unless MV::JSON_STRING_ESCAPES.size == 37
# A signed value still verifies to its message, escapes included.
secret = "s" * 64
["user-42", session, "a\"b<c>&d\\e\n"].each do |message|
  signed = MV.generate(secret, "signed cookie", message, "cookie.x", true)
  raise "FAILED: #{message.inspect} does not verify" unless MV.verified(secret, "signed cookie", signed, "cookie.x", true) == message
  count += 1
end
puts "ALL OK #{count}"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 3866"), "{stdout}");
}
