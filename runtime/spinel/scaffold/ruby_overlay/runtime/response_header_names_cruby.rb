# frozen_string_literal: true
# The response headers handed to Rack without redoing per-name work on every
# response, on CRuby/JRuby.
#
# - Rails' default headers in one pass. Base#commit_default_headers! asks the
#   header store for each of the five default headers and then writes a
#   missing one through `[]=`: two linear lookups, two pattern checks and a
#   downcased copy per name. The names and values are constants that pass
#   the checks, so here each name is looked up once, through the store's own
#   `index_of` (a name the app already set, in any case, keeps the app's
#   value), and a missing one is appended as `store_value` appends it: the
#   same key and value, in the same order, its downcased form a constant.
#   The five pairs are the ones in runtime/ruby/action_controller/
#   default_headers.rb; the test compares the two, so a change there that is
#   not made here fails it. The store is read from `@_headers`: every tree
#   this overlay ships in has the controller state renamed (src/project.rs
#   `controller_state_ivars`), as action_dispatch_request.rb reads
#   `@_request`; not through `headers`, which an app controller may define
#   for itself. A store of another class (or none) takes the shared method.
# - Each header name downcased for Rack once per name. Main.rack_response
#   keyed Rack's headers by `k.to_s.downcase`: a new String per header per
#   response, which `Hash#[]=` then looked up in the interned-string table
#   (0.8% of a page in the profile, that one line). The answer depends on
#   the name only, so it is kept, per name, in a frozen Hash that a miss
#   replaces whole (a lost race only recomputes), up to RACK_NAMES_CAP names.
#   The kept name is the interned String itself (`-`), the object the Hash
#   would have stored. Only plain UTF-8 String names are kept, so the key's
#   encoding is the one `downcase` gives; anything else is downcased as
#   before.
module ActionController
  class HeaderStore
    # `store_value` for a name `index_of` did not find, its downcased form given.
    def rh_append(key, lower, value)
      @keys << key
      @lower << lower
      @vals << value
      nil
    end
  end

  module DefaultHeadersOnePass
    def commit_default_headers!
      store = @_headers
      return super unless store.instance_of?(HeaderStore)
      store.rh_append("X-Frame-Options", "x-frame-options", "SAMEORIGIN") if store.index_of("X-Frame-Options") < 0
      store.rh_append("X-XSS-Protection", "x-xss-protection", "0") if store.index_of("X-XSS-Protection") < 0
      store.rh_append("X-Content-Type-Options", "x-content-type-options", "nosniff") if store.index_of("X-Content-Type-Options") < 0
      if store.index_of("X-Permitted-Cross-Domain-Policies") < 0
        store.rh_append("X-Permitted-Cross-Domain-Policies", "x-permitted-cross-domain-policies", "none")
      end
      if store.index_of("Referrer-Policy") < 0
        store.rh_append("Referrer-Policy", "referrer-policy", "strict-origin-when-cross-origin")
      end
      nil
    end
  end
  Base.prepend(DefaultHeadersOnePass) if defined?(Base)

  RACK_NAMES_CAP = 256
  @rh_rack_names = nil

  def self.rack_header_name(name)
    return name.to_s.downcase unless name.instance_of?(String) && name.encoding == Encoding::UTF_8
    names = @rh_rack_names
    unless names.nil?
      kept = names[name]
      return kept unless kept.nil?
    end
    kept = -name.downcase
    if names.nil? || names.size < RACK_NAMES_CAP
      @rh_rack_names = (names.nil? ? {} : names).merge(name => kept).freeze
    end
    kept
  end
end
