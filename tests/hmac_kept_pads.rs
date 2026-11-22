//! The CRuby/JRuby HMAC (runtime/spinel/message_digest_cruby.rb) built from
//! two keyed digest states kept per thread (K ^ ipad and K ^ opad absorbed,
//! RFC 2104) instead of resetting a kept `OpenSSL::HMAC`: every answer equals
//! `OpenSSL::HMAC`'s (SHA1 hex, SHA256 hex and raw, encodings included) for
//! keys shorter than, equal to and longer than a block, in bytes and in
//! multibyte UTF-8 (the same bytes answer the same), and messages around the
//! block edges; a key's states are built the second time a thread signs with
//! it (the first time only the key's hash is noted, never the key), are taken
//! from then on (a later call finds the same pair) and never consumed (every
//! call answers the same); a key String changed after use gets its own
//! states; a String subclass key is not kept; each thread keeps its own
//! tables; keys used once evict no kept states; each table stays at its cap,
//! the oldest entry leaving first; a digest without a 64-byte block goes
//! through `OpenSSL::HMAC`, and so does every call on an openssl whose digest
//! cannot finish a copied state; and the helpers are private.

use std::path::Path;
use std::process::Command;

#[path = "support/prerequisite.rs"]
mod prerequisite;

pub const SCRIPT: &str = r##"
require "openssl"
require_relative "runtime/spinel/message_digest_cruby"
check = ->(what, cond) { raise "FAILED: #{what}" unless cond }
pads = ->(digest, key) { MessageDigest.send(:hmac_pads, digest, key) }
e = 0xE9.chr(Encoding::UTF_8)
rng = Random.new(2026)
keys = [0, 1, 20, 32, 63, 64, 65, 100, 200].map { |n| rng.bytes(n) }
keys += [e * 10, e * 40, (e * 10).b, (e * 40).b]
msgs = ["", "a", "x" * 55, "y" * 56, "z" * 64, "w" * 1000, "caf" + e, rng.bytes(77)]
n = 0
2.times do
  keys.each do |key|
    msgs.each do |msg|
      h1 = MessageDigest.hmac_sha1_hex(key, msg)
      w1 = OpenSSL::HMAC.hexdigest("SHA1", key, msg)
      check.("sha1 hex k#{key.bytesize} m#{msg.bytesize}", h1 == w1 && h1.encoding == w1.encoding)
      h2 = MessageDigest.hmac_sha256_hex(key, msg)
      w2 = OpenSSL::HMAC.hexdigest("SHA256", key, msg)
      check.("sha256 hex k#{key.bytesize} m#{msg.bytesize}", h2 == w2 && h2.encoding == w2.encoding)
      r2 = MessageDigest.hmac_sha256(key, msg)
      wr = OpenSSL::HMAC.digest("SHA256", key, msg)
      check.("sha256 raw k#{key.bytesize} m#{msg.bytesize}", r2 == wr && r2.encoding == wr.encoding)
      n += 1
    end
  end
end
[[e * 10, (e * 10).b], [e * 40, (e * 40).b]].each do |u, b|
  check.("utf-8 and binary twins (#{u.bytesize} bytes)", MessageDigest.hmac_sha256_hex(u, "m") == MessageDigest.hmac_sha256_hex(b, "m"))
end
check.("another digest", MessageDigest.send(:keyed_hmac_raw, "SHA512", "key", "m") == OpenSSL::HMAC.digest("SHA512", "key", "m"))
check.("helpers private", !MessageDigest.respond_to?(:hmac_pads) && !MessageDigest.respond_to?(:keyed_hmac_raw))
puts "ALL OK #{n} #{RUBY_ENGINE} #{RUBY_VERSION} openssl #{OpenSSL::VERSION}"

# The kept states, where this openssl's digests finish a copied one. Where
# they do not, every answer above came from `OpenSSL::HMAC` and none is kept.
unless MessageDigest::HMAC_BY_HAND
  puts "NO KEPT STATES"
  exit
end
table = ->(name) { Thread.current.thread_variable_get(name)["SHA256"] }
first_pair = pads.("SHA256", keys[5])
check.("the states are kept", !first_pair.nil? && pads.("SHA256", keys[5]).equal?(first_pair))
check.("one table per digest", !pads.("SHA1", keys[5]).nil? && !pads.("SHA1", keys[5]).equal?(first_pair))

fresh = "fresh-" + rng.bytes(8).unpack1("H*")
check.("a first call builds no states", pads.("SHA256", fresh).nil? && !table.(:rh_hmac_pads).key?(fresh))
check.("a first call notes the key's hash, not the key", table.(:rh_hmac_once).key?(fresh.hash) && table.(:rh_hmac_once).keys.all?(Integer))
second = pads.("SHA256", fresh)
check.("the second call builds them", !second.nil? && table.(:rh_hmac_pads).key?(fresh) && !table.(:rh_hmac_once).key?(fresh.hash))
check.("a later call finds them", pads.("SHA256", fresh).equal?(second))
other = "other-" + rng.bytes(8).unpack1("H*")
check.("a first call under one digest is no first call under the other", pads.("SHA1", other).nil? && pads.("SHA256", other).nil?)

k = +("k" * 64)
first = MessageDigest.hmac_sha256_hex(k, "m")
check.("the key's states are kept before it changes", MessageDigest.hmac_sha256_hex(k, "m") == first && table.(:rh_hmac_pads).key?("k" * 64))
k.replace("j" * 64)
check.("a key changed after use", 3.times.all? { MessageDigest.hmac_sha256_hex(k, "m") == OpenSSL::HMAC.hexdigest("SHA256", "j" * 64, "m") })
check.("the old key still answers", MessageDigest.hmac_sha256_hex("k" * 64, "m") == first)

sub = Class.new(String).new("sub-key-old")
check.("a String subclass key", 3.times.all? { MessageDigest.hmac_sha256_hex(sub, "m") == OpenSSL::HMAC.hexdigest("SHA256", "sub-key-old", "m") })
check.("a String subclass key is not kept", !Thread.current.thread_variable_get(:rh_hmac_pads)["SHA256"].keys.any? { |x| !x.instance_of?(String) })
sub.replace("sub-key-new")
check.("a String subclass key changed after use", MessageDigest.hmac_sha256_hex(sub, "m") == OpenSSL::HMAC.hexdigest("SHA256", "sub-key-new", "m"))

mine = Thread.current.thread_variable_get(:rh_hmac_pads)
tables = 4.times.map do |t|
  Thread.new do
    got = 3.times.map { MessageDigest.hmac_sha1_hex("t#{t}" * 20, "msg") }
    [got, Thread.current.thread_variable_get(:rh_hmac_pads)]
  end
end.map(&:value)
tables.each_with_index do |(got, kept_there), t|
  check.("thread #{t}", got.uniq == [OpenSSL::HMAC.hexdigest("SHA1", "t#{t}" * 20, "msg")])
  check.("thread #{t} has its own table", !kept_there.equal?(mine) && kept_there["SHA1"].keys == ["t#{t}" * 20])
end
check.("the threads' keys stayed out of this thread's table", !mine["SHA1"].key?("t0" * 20))
noted_here = "noted-" + rng.bytes(8).unpack1("H*")
pads.("SHA1", noted_here)
check.("a key noted on one thread is new to another", Thread.new { MessageDigest.send(:hmac_pads, "SHA1", noted_here).nil? }.value)

kept_before = table.(:rh_hmac_pads).keys
200.times { |i| MessageDigest.hmac_sha256_hex("once-#{i}", "m") }
check.("keys used once evict no kept states", table.(:rh_hmac_pads).keys == kept_before)
check.("the once table stays at its cap, the oldest mark out first (#{table.(:rh_hmac_once).size})",
       table.(:rh_hmac_once).keys == (136..199).map { |i| "once-#{i}".hash })
80.times { |i| 2.times { MessageDigest.hmac_sha256_hex("key-#{i}", "m") } }
kept = table.(:rh_hmac_pads).keys
check.("cap held, the oldest states left first (#{kept.size})", kept == (16..79).map { |i| "key-#{i}" })
check.("after the cap", 3.times.all? { MessageDigest.hmac_sha256_hex("key-3", "m") == OpenSSL::HMAC.hexdigest("SHA256", "key-3", "m") })
puts "KEPT STATES OK"
"##;

/// An openssl whose digest does not finish a copied state: a copy raises,
/// an original finishes.
const NO_FINISH_ON_A_COPY: &str = r##"
require "openssl"
OpenSSL::Digest.prepend(Module.new do
  def initialize_copy(*)
    super
    @copied = true
  end

  def finish(*)
    raise "a copied digest does not implement finish()" if @copied
    super
  end
end)
whole = OpenSSL::Digest.new("SHA256")
whole.update("abc")
raise "FAILED: the stand-in broke an original digest" unless whole.send(:finish) == OpenSSL::Digest.digest("SHA256", "abc")
"##;

/// The answers are compared on every openssl. The kept states are read
/// only where its digests finish a copied one (`HMAC_BY_HAND`): on another
/// that half is a missing prerequisite, so a run that requires them fails.
#[test]
fn hmac_from_kept_pads_answers_as_openssl_hmac() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", SCRIPT]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 208"), "{stdout}");
    if stdout.contains("NO KEPT STATES") {
        prerequisite::missing(
            "an openssl whose digests finish a copied state (MessageDigest::HMAC_BY_HAND): the kept-state half did not run",
        );
        return;
    }
    assert!(stdout.contains("KEPT STATES OK"), "the kept-state half did not reach its end:\n{stdout}");
}

/// `Digest::Instance` gives every digest class a private `finish` that only
/// raises, so the method's presence says nothing: on an openssl whose digest
/// does not finish a copied state (here a copy raises, an original
/// finishes), every HMAC goes through `OpenSSL::HMAC` and no state is kept.
/// The first test's answers hold there too, and its kept-state half stays out.
#[test]
fn hmac_goes_through_openssl_when_a_digest_cannot_finish_a_copy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = &format!(
        "{NO_FINISH_ON_A_COPY}{}",
        r##"
require_relative "runtime/spinel/message_digest_cruby"
raise "FAILED: by hand on a digest that cannot finish" if MessageDigest::HMAC_BY_HAND
3.times do
  raise "FAILED: sha1" unless MessageDigest.hmac_sha1_hex("key", "msg") == OpenSSL::HMAC.hexdigest("SHA1", "key", "msg")
  raise "FAILED: sha256 hex" unless MessageDigest.hmac_sha256_hex("key", "msg") == OpenSSL::HMAC.hexdigest("SHA256", "key", "msg")
  raise "FAILED: sha256" unless MessageDigest.hmac_sha256("k" * 100, "msg") == OpenSSL::HMAC.digest("SHA256", "k" * 100, "msg")
end
raise "FAILED: states were kept" unless Thread.current.thread_variable_get(:rh_hmac_pads).nil?
puts "ALL OK"
"##
    );
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK"), "{stdout}");

    let answers = &format!("{NO_FINISH_ON_A_COPY}{SCRIPT}");
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", answers]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 208") && stdout.contains("NO KEPT STATES"), "{stdout}");
}
