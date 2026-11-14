# frozen_string_literal: true
# A path or route capture with nothing to decode, copied in one step on
# CRuby/JRuby.
#
# Router.decode_capture rebuilds its input byte by byte (an Integer#chr and
# a String concatenation per byte) so that every target can validate UTF-8
# the same way. An ASCII-only string without a `%` decodes to its own bytes
# as UTF-8, which is a copy: the answer is a new, unfrozen UTF-8 String
# with those bytes, as the shared walk gives. Every GET's path goes through
# it (PublicFiles.lookup), and every route capture. Anything else takes the
# shared walk, errors included.
module ActionDispatch
  module RouterDecodeCopy
    def decode_capture(value)
      if value.instance_of?(String) && value.ascii_only? && !value.include?("%")
        return String.new(value, encoding: Encoding::UTF_8)
      end
      super
    end
  end
  Router.singleton_class.prepend(RouterDecodeCopy) if defined?(Router)
end
