# frozen_string_literal: true
# The runtime's two substring searches through String#index and #rindex, on
# CRuby/JRuby.
#
# ActionController.find_substr and find_last walk a String one position at
# a time and cut a substring at each (`hay[i, n].to_s == needle`), the one
# search every target emits. They read the host out of every URL a redirect
# is checked against (location_host: five searches) and out of every
# absolute URL a `_url` helper builds (build_host_url with url_port_of:
# three): 2.2 us and 65 objects for `http://127.0.0.1:4411/articles/1`.
# In two ASCII-only Strings a character is a byte and the two always
# compare, so String#index and #rindex give the walk's answer in one call.
# Anything else takes the walk.
module ActionController
  module SubstringSearchInOneCall
    def find_substr(hay, needle)
      return super unless hay.instance_of?(String) && needle.instance_of?(String) && hay.ascii_only? && needle.ascii_only?
      hay.index(needle) || -1
    end

    def find_last(hay, needle)
      return super unless hay.instance_of?(String) && needle.instance_of?(String) && hay.ascii_only? && needle.ascii_only?
      hay.rindex(needle) || -1
    end
  end
  singleton_class.prepend(SubstringSearchInOneCall)
end
