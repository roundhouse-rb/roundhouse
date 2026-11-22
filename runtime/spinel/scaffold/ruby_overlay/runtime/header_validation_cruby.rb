# frozen_string_literal: true
# CRuby/JRuby's fast implementation of the shared header policy. Keep regex
# matching in this overlay: strict-target String runtimes cannot emit match?.
#
# The patterns are byte patterns. A binary copy (`.b`) is what lets them
# read a string whose bytes are not valid in its encoding without raising,
# but an ASCII-only string's characters are its bytes already: it is
# matched as it is, and only other strings are copied. Every header name
# and nearly every value is ASCII, and a response sets a dozen of them.
# The copies get Regexp objects of their own: CRuby recompiles a regexp in
# place when the encoding it is matched in changes, and one object shared
# by 7-bit strings and binary copies would recompile back and forth on
# every request that sends a non-ASCII byte.
module ActionController
  HEADER_KEY_ILLEGAL = /[\x00-\x1f\x7f": ]/.freeze
  HEADER_VALUE_ILLEGAL = /[\x00-\x08\x0a-\x1f\x7f]/.freeze
  HEADER_KEY_ILLEGAL_BYTES = Regexp.new(HEADER_KEY_ILLEGAL.source).freeze
  HEADER_VALUE_ILLEGAL_BYTES = Regexp.new(HEADER_VALUE_ILLEGAL.source).freeze

  def self.header_key_ok?(k)
    return false if k.nil?
    return false if k.length == 0
    k.ascii_only? ? !k.match?(HEADER_KEY_ILLEGAL) : !k.b.match?(HEADER_KEY_ILLEGAL_BYTES)
  end

  def self.header_value_ok?(v)
    return false if v.nil?
    v.ascii_only? ? !v.match?(HEADER_VALUE_ILLEGAL) : !v.b.match?(HEADER_VALUE_ILLEGAL_BYTES)
  end
end
