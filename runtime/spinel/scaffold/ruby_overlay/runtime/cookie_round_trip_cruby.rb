# frozen_string_literal: true
# A request's cookie round trip, once per distinct input, on CRuby/JRuby.
#
# A browser resends the same Cookie header on every request, and
# CgiIo.parse_cookies splits, strips and url-decodes it each time. Each
# thread keeps the last header it parsed with the answer; a request that
# carries the same header gets a new Hash of new Strings with the same
# contents (encodings included), so no caller ever holds what another
# request holds, and a caller that mutates its Hash or a value changes
# nothing for the next one.
#
# A permanent cookie's Expires (CookieJar.permanent_expires) is
# `20.years.from_now` at second resolution: every write within one second
# formats the same date. It is formatted once per second of `Time.now` (the
# clock the shared method reads, which `travel_to` stubs), and a text whose
# computation straddled a tick is not kept.
module CgiIoCookieMemo
  def parse_cookies(header)
    return super unless header.is_a?(String) && !header.empty?
    memo = Thread.current[:rh_cookie_memo]
    return memo[1].transform_values(&:dup) if !memo.nil? && memo[0] == header
    parsed = super
    key = header.frozen? ? header : header.dup.freeze
    Thread.current[:rh_cookie_memo] = [key, parsed.transform_values(&:dup).freeze].freeze
    parsed
  end
end
CgiIo.singleton_class.prepend(CgiIoCookieMemo) if defined?(CgiIo)

module CookieJarPermanentExpiresMemo
  def permanent_expires
    second = Time.now.to_i
    memo = @rh_permanent_expires
    return memo[1].dup if !memo.nil? && memo[0] == second
    text = super
    @rh_permanent_expires = [second, text.dup.freeze].freeze if Time.now.to_i == second
    text
  end
end
ActionController::CookieJar.singleton_class.prepend(CookieJarPermanentExpiresMemo) if defined?(ActionController::CookieJar)
