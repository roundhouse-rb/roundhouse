# frozen_string_literal: true
# Keyed-digest primitives for the CRuby/JRuby trees — the OpenSSL half
# of the two-layer split. `runtime/message_digest.rb` in the spinel tree
# is the same surface over sp_crypto's FFI; `ruby_runtime_files` swaps
# this file in at that path (same contract as db_cruby.rb → db.rb), so
# the framework Ruby above it (action_controller/message_verifier.rb)
# is written once and compiles for every ruby-family target.
#
# The surface is exactly what reproducing Rails' signed messages needs,
# and no more:
#
#   * HMAC-SHA1  — the digest Rails signs COOKIES with
#                  (`signed_cookie_digest || "SHA1"`; no `load_defaults`
#                  version sets `cookies_digest`, 8.2 included).
#   * HMAC-SHA256 — the digest Rails signs IDs with (`signed_id` /
#                  `find_signed!` are generated with digest: "SHA256").
#   * PBKDF2-HMAC-SHA256 — key derivation, SHA-256 since
#                  `load_defaults 7.0` set key_generator_hash_digest_class.
#
# Both digests are needed because Rails genuinely uses both; see the
# comment in message_verifier.rb for the full table. CSRF adds raw
# HMAC-SHA256, `secure_random_bytes`, and constant-time compare. The
# encrypted cookie jar adds AES-256-GCM.
require "openssl"
require "securerandom"

module MessageDigest
  # HMAC-SHA1(key, msg) as 40-char lowercase hex (BINARY, as
  # `OpenSSL::HMAC#hexdigest` answers it).
  def self.hmac_sha1_hex(key, msg)
    keyed_hmac_raw("SHA1", key, msg).unpack1("H*").force_encoding(Encoding::BINARY)
  end

  # HMAC-SHA256(key, msg) as 64-char lowercase hex (BINARY).
  def self.hmac_sha256_hex(key, msg)
    keyed_hmac_raw("SHA256", key, msg).unpack1("H*").force_encoding(Encoding::BINARY)
  end

  # Raw HMAC-SHA256 bytes (CSRF global token, not the hex cookies use).
  def self.hmac_sha256(key, msg)
    keyed_hmac_raw("SHA256", key, msg)
  end

  # HMAC(key, msg) = H((K ^ opad) || H((K ^ ipad) || msg)), RFC 2104, with
  # the two keyed states kept per thread: the digest after absorbing
  # K ^ ipad and the digest after absorbing K ^ opad depend on the key
  # only. Each call copies the two states and finishes them. Per call
  # (SHA1 and SHA256, 64-byte key, 168-byte message, one core of a
  # laptop, CRuby 4.0.9 with YJIT, OpenSSL 3.5): a new `OpenSSL::HMAC`
  # 3.1 us; a kept one reset 1.56 us, nearly all of it `HMAC#reset`
  # re-creating the signing context through OpenSSL 3's provider (93.7%
  # of this method on the blog's profile); a kept one copied 1.3 us; the
  # two kept digest states copied and finished 0.64 us.
  #
  # The SHA1 keys are the cookie jar's derived secrets, a handful per
  # process, and SHA256 has its own few (the signed-id secret, the query
  # cache's constant key). SHA256 also sees keys used once: each
  # session's CSRF secret (a token minted elsewhere is checked against
  # it) and Active Storage's per-size key. So a key's states are built
  # the second time the thread signs with it: the first time only the
  # key's `hash` is noted and `OpenSSL::HMAC` answers, as it did before,
  # and a key used once costs what it cost, keeps no copy of itself and
  # evicts no kept state. Each digest has its own two tables, each
  # capped, the oldest entry leaving first. A thread variable (not
  # `Thread.current[]`, which is per fiber: under Falcon a fiber serves
  # one keep-alive connection, so the states would be rebuilt for every
  # connection). Nothing may update a kept state:
  # it would not heal, and every HMAC for that key on that thread would be
  # wrong (refused, never forgeable) until its entry leaves the table; the
  # helpers below are private, and only this file reads the tables. Only a
  # plain String key is kept (a subclass is stored by reference, and a
  # later change to it could find the old key's states); any other key,
  # any digest without a 64-byte block, and an openssl whose digest does
  # not finish a copied state (`finish` is private on CRuby's, public on
  # JRuby's: `__send__` reaches both) go through `OpenSSL::HMAC`.
  HMAC_BLOCK_BYTES = { "SHA1" => 64, "SHA256" => 64 }.freeze
  HMAC_PADS_CAP = 64
  # Asked of a digest, not of the method's name: `Digest::Instance` gives
  # every digest class a private `finish` that only raises. A copy of an
  # updated state must finish to SHA256("abc"), the standard's own test
  # vector, and leave the original whole.
  HMAC_BY_HAND = begin
    probe = OpenSSL::Digest.new("SHA256")
    probe.update("abc")
    want = ["ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"].pack("H*")
    probe.dup.__send__(:finish) == want && probe.__send__(:finish) == want
  rescue StandardError, NotImplementedError
    false
  end

  def self.keyed_hmac_raw(digest, key, msg)
    return OpenSSL::HMAC.digest(digest, key, msg) unless HMAC_BY_HAND && key.instance_of?(String)
    pads = hmac_pads(digest, key)
    return OpenSSL::HMAC.digest(digest, key, msg) if pads.nil?
    inner = pads[0].dup
    inner.update(msg)
    outer = pads[1].dup
    outer.update(inner.__send__(:finish))
    outer.__send__(:finish)
  end

  # [K ^ ipad absorbed, K ^ opad absorbed] for `digest` and `key`, kept
  # per thread from the second call with that key; nil the first time,
  # and for a digest without a known block size.
  def self.hmac_pads(digest, key)
    block = HMAC_BLOCK_BYTES[digest]
    return nil if block.nil?
    thread = Thread.current
    cache = thread.thread_variable_get(:rh_hmac_pads) || thread.thread_variable_set(:rh_hmac_pads, {})
    by_key = cache[digest] ||= {}
    pads = by_key[key]
    return pads unless pads.nil?
    seen = thread.thread_variable_get(:rh_hmac_once) || thread.thread_variable_set(:rh_hmac_once, {})
    once = seen[digest] ||= {}
    mark = key.hash
    if once.delete(mark).nil?
      once.shift if once.size >= HMAC_PADS_CAP
      once[mark] = true
      return nil
    end
    by_key.shift if by_key.size >= HMAC_PADS_CAP
    k = key.bytesize > block ? OpenSSL::Digest.digest(digest, key) : key.b
    k = k.ljust(block, "\0".b)
    inner = OpenSSL::Digest.new(digest)
    inner.update(k.bytes.map { |b| b ^ 0x36 }.pack("C*"))
    outer = OpenSSL::Digest.new(digest)
    outer.update(k.bytes.map { |b| b ^ 0x5c }.pack("C*"))
    by_key[key] = [inner, outer].freeze
  end
  private_class_method :keyed_hmac_raw, :hmac_pads

  def self.secure_random_bytes(n)
    SecureRandom.random_bytes(n)
  end

  def self.secure_compare(a, b)
    return false if a.bytesize != b.bytesize
    diff = 0
    i = 0
    while i < a.bytesize
      diff = diff | (a.getbyte(i) ^ b.getbyte(i))
      i += 1
    end
    diff == 0
  end

  # PBKDF2-HMAC-SHA256 as RAW BYTES (not hex, not base64): the derived
  # key is the HMAC key above, and HMAC keys are bytes.
  def self.pbkdf2_sha256(secret, salt, iters, dklen)
    OpenSSL::PKCS5.pbkdf2_hmac(secret, salt, iters, dklen, OpenSSL::Digest::SHA256.new)
  end

  # AES-256-GCM, the cipher Rails' encrypted cookie jar (and so its
  # session store) uses since `use_authenticated_cookie_encryption`:
  # a 32-byte key, a 12-byte IV, no associated data, a 16-byte tag.
  #
  # `aes_256_gcm_encrypt` answers the ciphertext followed by the tag
  # (GCM is a stream mode, so the tag is always the last 16 bytes).
  # `aes_256_gcm_decrypt` answers the plaintext, or "" when the tag does
  # not authenticate the ciphertext under this key and IV. The callers
  # encrypt a JSON envelope, never an empty message, so "" cannot be a
  # real plaintext here.
  def self.aes_256_gcm_encrypt(key, iv, plain)
    c = OpenSSL::Cipher.new("aes-256-gcm")
    c.encrypt
    c.key = key
    c.iv = iv
    c.auth_data = ""
    ct = c.update(plain) + c.final
    ct + c.auth_tag
  end

  # A tag that is not 16 bytes is refused before the cipher sees it, as
  # Rails' MessageEncryptor refuses it: OpenSSL would check a truncated
  # tag against only as many bytes as it was given.
  def self.aes_256_gcm_decrypt(key, iv, ct, tag)
    return "" if tag.bytesize != 16
    c = OpenSSL::Cipher.new("aes-256-gcm")
    c.decrypt
    c.key = key
    c.iv = iv
    c.auth_tag = tag
    c.auth_data = ""
    c.update(ct) + c.final
  rescue OpenSSL::Cipher::CipherError
    ""
  end
end
