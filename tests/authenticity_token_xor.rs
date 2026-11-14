//! `ActionController::AuthenticityToken.xor`, the one-time-pad step of every
//! masked CSRF token (one per form and one for the csrf meta tag on each
//! page), answers the byte XOR of its two 32-byte arguments, leaves both
//! arguments as they were, answers a binary String whatever their
//! encodings, and builds
//! one String per call: it writes over a copy of the first argument's bytes,
//! where appending a `chr` per byte built 64 strings and copied the growing
//! one 32 times.

use std::path::Path;
use std::process::Command;

#[path = "support/emit_and_run.rs"]
mod emit_and_run;
#[path = "support/prerequisite.rs"]
mod prerequisite;

/// The same proof on the compiled targets, inside an emitted tree: the XOR
/// of pads with a NUL at the first, a middle and the last byte (a copy sized
/// up to the first NUL would lose the tail), and 1,000 tokens masked from
/// fresh random pads, about one in eight of which holds a NUL, that all
/// verify. Spinel compiles this runtime too, and a single request passing
/// its CSRF check would prove nothing: seven tokens in eight carry no NUL.
const XOR_PROBE: &str = r##"
at = ActionController::AuthenticityToken
secret = "\x0F" * 32
pads = ["\0" + ("\x80" * 31), ("\x80" * 15) + "\0" + ("\x7F" * 16), ("\xFF" * 31) + "\0"]
n = 0
pads.each do |pad|
  before = pad.bytes
  got = at.xor(pad, secret)
  ok = got.bytesize == 32 && pad.bytes == before
  i = 0
  while i < 32
    ok = false if got.getbyte(i) != (before[i] ^ 15)
    i += 1
  end
  raise "FAILED: vector " + n.to_s unless ok
  puts "PASS vector " + n.to_s
  n += 1
end
verified = 0
with_nul = 0
k = 0
while k < 1000
  raw = MessageDigest.secure_random_bytes(32)
  pad = MessageDigest.secure_random_bytes(32)
  j = 0
  nul = false
  while j < 32
    nul = true if pad.getbyte(j) == 0
    j += 1
  end
  with_nul += 1 if nul
  token = Base64.urlsafe_encode64_nopad(pad + at.xor(pad, raw))
  verified += 1 if at.valid?(token, Base64.urlsafe_encode64_nopad(raw))
  k += 1
end
puts "tokens verified " + verified.to_s + "/1000, pads with a NUL " + with_nul.to_s
raise "FAILED: " + (1000 - verified).to_s + " tokens did not verify" unless verified == 1000
raise "FAILED: no pad held a NUL" unless with_nul > 0
puts "ALL OK"
"##;

#[test]
fn xor_probe_with_nul_pads_and_a_thousand_tokens_on_cruby() {
    let run = emit_and_run::real_blog().run_ruby(XOR_PROBE);
    run.assert_passes();
    assert!(run.stdout.contains("ALL OK"), "{}", run.stdout);
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn xor_probe_with_nul_pads_and_a_thousand_tokens_on_spinel() {
    let run = emit_and_run::real_blog().run_spinel(XOR_PROBE);
    run.assert_passes();
    assert!(run.stdout.contains("ALL OK"), "{}", run.stdout);
}

#[test]
fn authenticity_token_xor_answers_bytewise_without_a_string_per_byte() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
module ActionController
  def self.set_forgery_flag(_on) = nil
end
require_relative "runtime/ruby/action_controller/authenticity_token"
at = ActionController::AuthenticityToken
reference = ->(a, b) { a.bytes.first(32).zip(b.bytes.first(32)).map { |x, y| x ^ y }.pack("C*") }
rng = Random.new(20261109)
zero = ("\0" * 32).b
full = ("\xFF" * 32).b
ascii = "a" * 32
pairs = [[zero, zero], [zero, full], [full, full], [ascii, ascii], [ascii.b, full], [full, ascii.b], [ascii.b, ascii],
         [ascii, full], [full, ascii]]
20_000.times { pairs << [rng.bytes(32), rng.bytes(32)] }
pairs.each do |a, b|
  a_before = a.dup
  b_before = b.dup
  got = at.xor(a, b)
  want = reference.(a, b)
  raise "FAILED: xor bytes for #{a.inspect}, #{b.inspect}: #{got.inspect}, want #{want.inspect}" unless got.bytes == want.bytes
  raise "FAILED: xor answered #{got.bytesize} bytes" unless got.bytesize == 32
  raise "FAILED: xor changed its arguments" unless a == a_before && b == b_before
  raise "FAILED: xor answered #{got.encoding}, want binary" unless got.encoding == Encoding::BINARY
end
pad = rng.bytes(32)
raw = rng.bytes(32)
masked = pad + at.xor(pad, raw)
raise "FAILED: unmasking does not give the secret back" unless at.xor(masked[0, 32], masked[32, 32]) == raw

a = rng.bytes(32)
b = rng.bytes(32)
1000.times { at.xor(a, b) }
before = GC.stat(:total_allocated_objects)
1000.times { at.xor(a, b) }
per_call = (GC.stat(:total_allocated_objects) - before).fdiv(1000)
# One per call, plus at most one for the whole measured loop.
raise "FAILED: xor allocates #{per_call} objects per call, want 1" unless per_call < 1.5
puts "ALL OK #{pairs.size}"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 20009"), "{stdout}");
}

/// The masking keeps its security properties across the change: every token
/// gets a fresh random pad, a token masked by the old per-byte XOR verifies
/// under the new unmask and a new token unmasks under the old XOR, and the
/// check is still the constant-time `secure_compare`, never `==`.
#[test]
fn masked_tokens_keep_fresh_pads_old_compatibility_and_constant_time_compare() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require "securerandom"
require_relative "runtime/spinel/base64"
module ActionController
  def self.set_forgery_flag(_on) = nil
  module Current
    def self.session = ($session ||= {})
  end
end
module MessageDigest
  def self.secure_random_bytes(n) = SecureRandom.random_bytes(n)
  def self.secure_compare(a, b)
    $compares += 1
    return false if a.bytesize != b.bytesize
    diff = 0
    a.bytesize.times { |i| diff |= a.getbyte(i) ^ b.getbyte(i) }
    diff == 0
  end
  def self.hmac_sha256(key, msg) = OpenSSL::HMAC.digest("SHA256", key, msg)
end
require "openssl"
require_relative "runtime/ruby/action_controller/authenticity_token"
at = ActionController::AuthenticityToken
old_xor = lambda do |a, b|
  out = ""
  32.times { |i| out = out + (a.getbyte(i) ^ b.getbyte(i)).chr }
  out
end
$compares = 0

tokens = Array.new(200) { at.masked }
secret = ActionController::Current.session[:_csrf_token].to_s
raw = Base64.urlsafe_decode64(secret)
raise "FAILED: a token is empty" if tokens.any?(&:empty?)
raise "FAILED: two tokens share a pad" unless tokens.map { |t| Base64.urlsafe_decode64(t)[0, 32] }.uniq.size == tokens.size
raise "FAILED: a token is the secret" if tokens.include?(secret)
raise "FAILED: a new token does not verify" unless tokens.all? { |t| at.valid?(t, secret) }
tokens.each do |t|
  d = Base64.urlsafe_decode64(t)
  raise "FAILED: the old XOR does not unmask a new token" unless old_xor.(d[0, 32], d[32, 32]) == raw
end
50.times do
  pad = SecureRandom.random_bytes(32)
  old_token = Base64.urlsafe_encode64_nopad(pad + old_xor.(pad, raw))
  raise "FAILED: a token masked by the old XOR does not verify" unless at.valid?(old_token, secret)
end
other = Base64.urlsafe_encode64_nopad(SecureRandom.random_bytes(32))
raise "FAILED: a token for another secret verifies" if at.valid?(tokens.first, other)
$compares = 0
at.valid?(tokens.first, secret)
raise "FAILED: valid? compared without secure_compare" unless $compares >= 1
# Accepted only through secure_compare: with it answering false, a right token is refused.
MessageDigest.define_singleton_method(:secure_compare) { |_a, _b| false }
raise "FAILED: valid? accepted a token without secure_compare" if at.valid?(tokens.first, secret)
puts "ALL OK"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK"), "{stdout}");
}
