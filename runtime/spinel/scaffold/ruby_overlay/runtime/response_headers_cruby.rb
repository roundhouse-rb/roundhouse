# frozen_string_literal: true
# The response headers a controller writes, assembled without a copy per
# header on CRuby/JRuby. The shared runtime spells both pieces below in the
# forms every target compiles; these are the same answers, encoding
# included (tests/header_validation.rs compares them):
#
# - HeaderStore#index_of downcases the name it is asked for, on every
#   read and every write (`store_value` looks the name up first). An ASCII
#   name is compared with `casecmp` instead, which folds ASCII letters
#   only: the stored names are downcased already, so the answer is the
#   same, and nothing is allocated. `casecmp` is a full C call where YJIT
#   compiles `==` inline, so a stored name of another byte length (which
#   can never compare equal) is passed over on its `bytesize` first.
#   Anything else takes the shared path.
# - ContentSecurityPolicy#build and #join_sources grow their result with
#   `out = out + piece`, a new String per piece. The same loop appends to
#   one buffer here (`<<` negotiates encodings as `+` does; a `to_s` that
#   answers something other than a String goes through `+` and raises as
#   the shared code does, where `<<` would append a codepoint).
module ActionController
  module HeaderStoreLookup
    def index_of(key)
      return super unless key.is_a?(String) && key.ascii_only?
      lower = @lower
      size = key.bytesize
      i = 0
      n = lower.length
      while i < n
        name = lower[i]
        if name.is_a?(String)
          return i if name.bytesize == size && key.casecmp(name) == 0
        elsif key.casecmp(name) == 0
          return i
        end
        i += 1
      end
      -1
    end
  end
  HeaderStore.prepend(HeaderStoreLookup) if defined?(HeaderStore)
end

module ActionDispatch
  module ContentSecurityPolicyBuffer
    def build
      names = @names
      n = names.length
      return "" if n == 0
      out = +""
      i = 0
      while i < n
        out << "; " if i > 0
        name = names[i].to_s
        out << (name.is_a?(String) ? name : +"" + name) << " " << join_sources(@values[i])
        i += 1
      end
      out
    end

    def join_sources(list)
      return super unless list.is_a?(Array)
      n = list.length
      return "" if n == 0
      out = +""
      i = 0
      while i < n
        out << " " if i > 0
        item = list[i]
        unless item.nil?
          text = item.to_s
          out << (text.is_a?(String) ? text : +"" + text)
        end
        i += 1
      end
      out
    end
  end
  ContentSecurityPolicy.prepend(ContentSecurityPolicyBuffer) if defined?(ContentSecurityPolicy)
end
