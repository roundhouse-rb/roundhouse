//! A page masks one CSRF token per form and one for the csrf meta tag, all
//! from the session's one secret: `AuthenticityToken.masked` decodes that
//! secret once per request, and a kept decode is answered only for the very
//! String it came from. Every token still verifies against its own session
//! and no other, across sessions that take turns on one thread, a session
//! replaced mid-request, two threads at once, and a secret that does not
//! decode.

use std::path::Path;
use std::process::Command;

#[path = "support/emit_and_run.rs"]
mod emit_and_run;
#[path = "support/prerequisite.rs"]
mod prerequisite;

/// On the compiled targets, inside an emitted tree: the kept decode answers
/// the same bytes as a fresh one, for a secret asked twice and for two
/// secrets taking turns.
const KEPT_SECRET_PROBE: &str = r##"
at = ActionController::AuthenticityToken
a = at.mint
b = at.mint
turns = [a, a, b, a, b, b, a]
i = 0
while i < turns.length
  s = turns[i]
  got = at.secret_bytes(s)
  raise "FAILED: turn " + i.to_s + " answers other bytes" unless got == at.decode(s) && got.length == 32
  i += 1
end
puts "ALL OK"
"##;

#[test]
fn kept_secret_answers_the_decode_in_the_emitted_app_on_cruby() {
    let run = emit_and_run::real_blog().run_ruby(KEPT_SECRET_PROBE);
    run.assert_passes();
    assert!(run.stdout.contains("ALL OK"), "{}", run.stdout);
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn kept_secret_answers_the_decode_in_the_emitted_app_on_spinel() {
    let run = emit_and_run::real_blog().run_spinel(KEPT_SECRET_PROBE);
    run.assert_passes();
    assert!(run.stdout.contains("ALL OK"), "{}", run.stdout);
}

#[test]
fn masked_decodes_the_session_secret_once_per_request_and_never_serves_another_sessions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require "securerandom"
require "openssl"
require_relative "runtime/spinel/base64"
module ActionController
  def self.set_forgery_flag(_on) = nil
  module Current
    def self.session = Thread.current[:test_session]
    def self.session=(s)
      Thread.current[:test_session] = s
    end
  end
end
module MessageDigest
  def self.secure_random_bytes(n) = SecureRandom.random_bytes(n)
  def self.secure_compare(a, b) = OpenSSL.fixed_length_secure_compare(a, b)
  def self.hmac_sha256(key, msg) = OpenSSL::HMAC.digest("SHA256", key, msg)
end
require_relative "runtime/ruby/action_controller/authenticity_token"
at = ActionController::AuthenticityToken
cur = ActionController::Current
$decodes = Hash.new(0)
at.singleton_class.prepend(Module.new do
  def decode(encoded)
    $decodes[encoded] += 1
    super
  end
end)
secret_of = ->(session) { session[:_csrf_token].to_s }

# One request: a session, six tokens (five forms and the meta tag).
one = {}
cur.session = one
tokens = Array.new(6) { at.masked }
s1 = secret_of.(one)
raise "FAILED: an empty token" if tokens.any?(&:empty?)
raise "FAILED: two tokens share a pad" unless tokens.uniq.size == 6
raise "FAILED: a token does not verify" unless tokens.all? { |t| at.valid?(t, s1) }
$decodes.clear
6.times { at.masked }
raise "FAILED: the secret was decoded #{$decodes[s1]} times for six more tokens of the request, want 0" unless $decodes[s1] == 0

# Sessions taking turns on the thread, as requests do: each token is its own session's.
two = {}
cur.session = two
t2 = at.masked
s2 = secret_of.(two)
raise "FAILED: two sessions minted one secret" if s1 == s2
raise "FAILED: a token of the second session does not verify" unless at.valid?(t2, s2)
raise "FAILED: a token of the second session verifies against the first" if at.valid?(t2, s1)
200.times do |i|
  session, secret, other = i.even? ? [one, s1, s2] : [two, s2, s1]
  cur.session = session
  t = at.masked
  raise "FAILED: turn #{i}: the token is not its session's" unless at.valid?(t, secret) && !at.valid?(t, other)
end

# The next request of the same session parses its cookie again: an equal
# secret in another String. Same answer, decoded again (kept by identity).
cur.session = one
at.masked
again = { _csrf_token: s1.dup }
cur.session = again
$decodes.clear
later = Array.new(3) { at.masked }
raise "FAILED: a new String of the secret was decoded #{$decodes[s1]} times for three tokens, want 1" unless $decodes[s1] == 1
raise "FAILED: an equal secret in a new String does not verify" unless later.all? { |t| at.valid?(t, s1) }

# A session swapped mid-request (`session=`): the next token follows it.
cur.session = one
before = at.masked
fresh = {}
cur.session = fresh
after = at.masked
s3 = secret_of.(fresh)
raise "FAILED: the token after a session swap is not the new session's" unless at.valid?(after, s3) && !at.valid?(after, s1)
raise "FAILED: the token before the swap changed owner" unless at.valid?(before, s1)

# The same session object with its secret removed and minted again, as
# after reset_session, which empties the session it has: the next token is
# the new secret's (a decode kept per session would not be).
cur.session = one
at.masked
one.delete(:_csrf_token)
renewed = at.masked
s4 = secret_of.(one)
raise "FAILED: the secret was not minted again" if s4 == s1 || s4.empty?
raise "FAILED: the token after the secret was replaced is not the new secret's" unless at.valid?(renewed, s4) && !at.valid?(renewed, s1)
one[:_csrf_token] = s1

# The secret's own String changed in place (`replace`): the same object,
# other bytes. The next tokens are the new bytes' (a decode kept by
# identity alone would answer the old ones), decoded once for the three.
three = {}
cur.session = three
at.masked
s5 = secret_of.(three)
was = s5.dup
s5.replace(Base64.urlsafe_encode64_nopad(SecureRandom.random_bytes(32)))
$decodes.clear
changed = Array.new(3) { at.masked }
raise "FAILED: a secret changed in place was decoded #{$decodes.values.sum} times for three tokens, want 1" unless $decodes.values.sum == 1
raise "FAILED: a token after an in-place change is not the new bytes'" unless changed.all? { |t| at.valid?(t, s5) && !at.valid?(t, was) }

# A secret that is not 32 bytes, or not Base64, masks to "" every time.
[{ _csrf_token: "c2hvcnQ" }, { _csrf_token: "!!!!" }].each do |bad|
  cur.session = bad
  3.times { raise "FAILED: a malformed secret masks to a token" unless at.masked == "" }
end
cur.session = one
raise "FAILED: a good session after a malformed one" unless at.valid?(at.masked, s1)

# No session: the empty token, and nothing kept from it.
cur.session = nil
raise "FAILED: a token without a session" unless at.masked == ""

# The copy is written over the one kept before: a decode that is kept
# builds the decode's own objects and the Array of the three, no String.
x, y = Array.new(2) { Base64.urlsafe_encode64_nopad(SecureRandom.random_bytes(32)) }
allocated = ->(&work) do
  1000.times(&work)
  before = GC.stat(:total_allocated_objects)
  1000.times(&work)
  (GC.stat(:total_allocated_objects) - before).fdiv(1000)
end
decode = allocated.call { at.decode(x) }
kept = allocated.call { at.secret_bytes(x); at.secret_bytes(y) }.fdiv(2)
raise "FAILED: a kept decode allocates #{(kept - decode).round(2)} objects more than the decode, want 1" unless kept - decode < 1.5
copy = Thread.current[:rh_csrf_secret][2]
at.secret_bytes(x)
raise "FAILED: a later decode kept a new String for its copy" unless Thread.current[:rh_csrf_secret][2].equal?(copy) && copy == x

# Two threads at once, each with its session: neither is served the other's secret.
errors = Queue.new
threads = [one, two].map do |session|
  Thread.new do
    cur.session = session
    secret = secret_of.(session)
    other = secret == s1 ? s2 : s1
    2000.times do
      t = at.masked
      errors << "a thread's token is not its session's" unless at.valid?(t, secret) && !at.valid?(t, other)
      Thread.pass
    end
  end
end
threads.each(&:join)
raise "FAILED: #{errors.pop}" unless errors.empty?
puts "ALL OK"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK"), "{stdout}");
}
