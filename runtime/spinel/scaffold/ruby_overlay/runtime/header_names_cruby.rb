# frozen_string_literal: true
# A request header's HTTP name mapped to its Rack env key once per name, on
# CRuby/JRuby.
#
# `request.headers["Accept-Encoding"]` goes through Headers.env_name, which
# upcases the name twice, swaps its dashes, matches two patterns and scans
# the CGI variable list on every read; an app reads the same few names on
# every request (campfire's cached responses read five). The answer depends
# on the name only, so it is kept, per name, in a frozen Hash that a miss
# replaces whole (a lost race only recomputes), up to ENV_NAMES_CAP names.
# Only plain UTF-8 String names are kept, so the key's encoding is the one
# the shared method would give; the kept key is frozen, which a caller that
# uses it as an env key (every caller in the runtime) cannot tell.
module ActionDispatch
  module Http
    module HeadersEnvNameMemo
      ENV_NAMES_CAP = 256

      def env_name(key)
        return super unless key.instance_of?(String) && key.encoding == Encoding::UTF_8
        names = @rh_env_names
        unless names.nil?
          name = names[key]
          return name unless name.nil?
        end
        name = super
        if names.nil? || names.size < ENV_NAMES_CAP
          kept = name.frozen? ? name : name.dup.freeze
          @rh_env_names = (names.nil? ? {} : names).merge(key.dup.freeze => kept).freeze
        end
        name
      end
    end
    Headers.singleton_class.prepend(HeadersEnvNameMemo) if defined?(Headers)
  end
end
