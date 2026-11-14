# frozen_string_literal: true
# Rails-style masked authenticity tokens: one session secret, a
# per-render one-time pad, XOR + URL-safe Base64. Turbo's
# `csrf-token` meta and `X-CSRF-Token` header consume the masked
# value unchanged.
#
# Write-once for every ruby-family target. Strict-target `base.rb`
# still answers an empty token (no `Current.session`); this file is
# required from `action_controller.rb`, off those tables, like
# `current.rb`, so only the ruby family (CRuby, JRuby, Spinel) compiles
# it. Masking is byte XOR, and the only primitives below are
# `MessageDigest.secure_random_bytes`, `hmac_sha256`, and
# `secure_compare`.
module ActionController
    module AuthenticityToken
    LENGTH = 32
    GLOBAL_SCOPE = "!real_csrf_token"

    def self.masked
      raw_b64 = ensure_session_token
      return "" if raw_b64.empty?
      raw = secret_bytes(raw_b64)
      return "" if raw.length != 32
      pad = MessageDigest.secure_random_bytes(32)
      return "" if pad.length != 32
      Base64.urlsafe_encode64_nopad(pad + xor(pad, raw))
    end

    def self.valid?(given, expected_b64)
      return false if expected_b64.empty?
      return false if given.empty?
      expected = decode(expected_b64)
      return false if expected.length != 32
      actual = unmask(given)
      return false if actual.length != 32
      return true if MessageDigest.secure_compare(actual, expected)
      MessageDigest.secure_compare(actual, hmac_global(expected))
    end

    def self.ensure_session_token
      session = Current.session
      return "" if session.nil?
      token = session[:_csrf_token]
      if token.nil?
        token = mint
        session[:_csrf_token] = token
      end
      token.to_s
    end

    def self.mint
      Base64.urlsafe_encode64_nopad(MessageDigest.secure_random_bytes(32))
    end

    def self.unmask(encoded)
      decoded = decode(encoded)
      n = decoded.length
      return decoded if n == 32
      return "" if n != 64
      xor(decoded[0, 32].to_s, decoded[32, 32].to_s)
    end

    # Not left to raise: a gem's `require "base64"` swaps in the strict stdlib decoder, and Rails reads its ArgumentError as a bad token.
    def self.decode(encoded)
      Base64.urlsafe_decode64(encoded)
    rescue ArgumentError
      ""
    end

    # The session secret's bytes. A page masks one token per form and one
    # for the csrf meta tag, all from the one secret its session holds:
    # the last decode is kept in `Thread.current[]` (per fiber on CRuby
    # and JRuby, per thread on Spinel) and answered again only for the
    # very String it was decoded from, which a session holds for one
    # request. Identity first: another session's secret, or the same one
    # parsed from the next request's cookie, is a different String and is
    # decoded, so a secret's bytes are only ever compared with the copy
    # taken from that same String, never with another session's secret.
    # A String changed in place (`replace`) differs from its copy and is
    # decoded again. The three stay until the next page's first token
    # replaces them; after the slot's first decode the copy is written
    # over the one kept before, so a decode builds no String for it.
    def self.secret_bytes(encoded)
      kept = Thread.current[:rh_csrf_secret]
      return kept[1].to_s if !kept.nil? && encoded.equal?(kept[0]) && encoded == kept[2]
      raw = decode(encoded)
      Thread.current[:rh_csrf_secret] = [encoded, raw, kept.nil? ? encoded.dup : kept[2].replace(encoded)]
      raw
    end

    # The XOR of two 32-byte strings (`masked` checks the pad's length,
    # `unmask` slices 32), written over a copy of `a`: one string, where
    # appending a `chr` per byte built 64 and copied the growing one each
    # time, for every masked token (a page renders one per form and one
    # for the csrf meta tag). The work does not depend on the bytes. The
    # copy is binary whatever `a`'s encoding, so `valid?` counts bytes
    # when it asks the length; a longer `a` would come back longer and
    # fail `unmask`'s length check, closed.
    def self.xor(a, b)
      out = a.b
      i = 0
      while i < 32
        out.setbyte(i, a.getbyte(i) ^ b.getbyte(i))
        i += 1
      end
      out
    end

    def self.hmac_global(raw)
      MessageDigest.hmac_sha256(raw, GLOBAL_SCOPE)
    end
  end

  def self.masked_authenticity_token
    AuthenticityToken.masked
  end

  def self.csrf_token_valid?(given, expected)
    AuthenticityToken.valid?(given, expected)
  end

  # Real masked tokens are available — turn the shared flag on. Extras
  # that omit this file keep the base.rb default (off) so stub-empty
  # tokens do not fail-closed every POST.
  set_forgery_flag(true)
end
