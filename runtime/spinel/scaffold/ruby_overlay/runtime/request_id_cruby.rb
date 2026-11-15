# frozen_string_literal: true
# A fresh request id written as one String, on CRuby/JRuby.
#
# RequestId.make answers `SecureRandom.uuid` for a request that carries no
# id of its own, which is every request a browser sends. The library takes
# sixteen random bytes, sets the version and variant bits and formats them
# through an Array of five numbers or five hex Strings (the form depends on
# the Ruby). Here the same sixteen bytes, asked of the same method
# (`SecureRandom.random_bytes`), get the same two bits set and are unpacked
# as one hex String, with the four dashes put in: the same id, in the same
# encoding, a new unfrozen String as before. A request that brings its own
# id takes the shared method.
#
# `SecureRandom.uuid` itself is not called, so what can stand in for it is
# asked: a test that stubs it goes through the emitted stub slot
# (runtime/secure_random_stub.rb, `STUB_UUID_ON`), and while the slot is on
# the shared method answers, with the stubbed id. An app that redefines
# `SecureRandom.uuid` in any other way is not asked for request ids.
module ActionDispatch
  module RequestIdOneString
    def make(value)
      return super unless blank?(value)
      return super if defined?(SecureRandom::STUB_UUID_ON) && SecureRandom::STUB_UUID_ON[0]
      bytes = SecureRandom.random_bytes(16)
      bytes.setbyte(6, (bytes.getbyte(6) & 0x0f) | 0x40)
      bytes.setbyte(8, (bytes.getbyte(8) & 0x3f) | 0x80)
      id = bytes.unpack1("H*")
      id.insert(20, "-")
      id.insert(16, "-")
      id.insert(12, "-")
      id.insert(8, "-")
      id
    end
  end
  RequestId.singleton_class.prepend(RequestIdOneString) if defined?(RequestId)
end
