//! `Base64.urlsafe_encode64_nopad` answers the URL-safe encoding without its
//! trailing `=`, for every input length, under the runtime's own encoder and
//! under the stdlib one a gem's `require "base64"` swaps in, and finds the
//! padding by byte: no one-character String per padding test (a masked CSRF
//! token is 64 bytes, two `=`, and a page encodes one per form).

use std::path::Path;
use std::process::Command;

#[path = "support/emit_and_run.rs"]
mod emit_and_run;
#[path = "support/prerequisite.rs"]
mod prerequisite;

const NOPAD_PROBE: &str = r##"
want = ["", "YQ", "YWI", "YWJj", "YWJjZA", "-_8", "_-7_", "AA"]
inputs = ["", "a", "ab", "abc", "abcd", "\xFB\xFF", "\xFF\xEE\xFF", "\0"]
i = 0
while i < inputs.length
  got = Base64.urlsafe_encode64_nopad(inputs[i])
  raise "FAILED: nopad of input " + i.to_s + " is " + got unless got == want[i]
  i += 1
end
puts "ALL OK"
"##;

#[test]
fn nopad_vectors_in_the_emitted_app_on_cruby() {
    let run = emit_and_run::real_blog().run_ruby(NOPAD_PROBE);
    run.assert_passes();
    assert!(run.stdout.contains("ALL OK"), "{}", run.stdout);
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn nopad_vectors_in_the_emitted_app_on_spinel() {
    let run = emit_and_run::real_blog().run_spinel(NOPAD_PROBE);
    run.assert_passes();
    assert!(run.stdout.contains("ALL OK"), "{}", run.stdout);
}

#[test]
fn nopad_matches_the_reference_for_every_length_and_tests_padding_by_byte() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require_relative "runtime/spinel/base64"
reference = ->(s) { [s].pack("m0").tr("+/", "-_").delete("=") }
rng = Random.new(20261110)
inputs = [""]
(1..130).each { |n| 8.times { inputs << rng.bytes(n) } }
inputs << ("=" * 7) << "=" << "a=" << ("\xFF" * 64) << ("\0" * 32)
check = lambda do |label|
  inputs.each do |s|
    got = Base64.urlsafe_encode64_nopad(s)
    raise "FAILED (#{label}): #{s.inspect} gives #{got.inspect}, want #{reference.(s).inspect}" unless got == reference.(s)
    raise "FAILED (#{label}): padding left on #{got.inspect}" if got.end_with?("=")
    raise "FAILED (#{label}): the answer does not decode back" unless Base64.urlsafe_decode64(got).bytes == s.bytes
  end
end
check.("runtime encoder")
# What happens in an emitted app on CRuby: a gem requires the stdlib module
# after the runtime's, and its encoder replaces the runtime's by name.
require "base64"
check.("stdlib encoder")
token = rng.bytes(64)
secret = rng.bytes(32)
1000.times { Base64.urlsafe_encode64_nopad(token) }
[[token, "a 64-byte token"], [secret, "a 32-byte secret"]].each do |s, label|
  before = GC.stat(:total_allocated_objects)
  1000.times { Base64.urlsafe_encode64_nopad(s) }
  per_call = (GC.stat(:total_allocated_objects) - before).fdiv(1000)
  # The encoding and the answer without padding; a String per padding test made it 4 and 5.
  raise "FAILED: nopad of #{label} allocates #{per_call} objects per call, want 2" unless per_call < 2.5
end
puts "ALL OK #{inputs.size}"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 1046"), "{stdout}");
}
