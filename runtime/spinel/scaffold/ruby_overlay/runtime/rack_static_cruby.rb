# frozen_string_literal: true
# Rack::Static passes a path it cannot serve to the app without unescaping
# and cleaning it first, on CRuby/JRuby.
#
# The emitted config.ru mounts Rack::Static for the asset URLs, so every
# request that reaches the app goes through its `call`, which unescapes the
# path, splits it on "/" and joins it again (Utils.clean_path_info) before
# asking `can_serve`; for a page that is not an asset the answer is no and
# the request goes to the app. For a plain path (ASCII, no "%", no empty or
# dot segment, no backslash) the unescaped, cleaned path is the path itself
# without a trailing "/" (or "/" for "" and "/"), so the instance's own
# `can_serve` is asked about that, and about the raw path when that differs
# (rack before 2.2.13, 3.0.14 and 3.1.12 asks about the raw one): when both
# are no, the request goes straight to the
# app, which is the branch Rack::Static takes. Any other path, and any path
# it may serve, takes Rack::Static's own `call`.
module RackStaticPlainMiss
  def call(env)
    path = env[Rack::PATH_INFO]
    if path.instance_of?(String) && path.ascii_only? && !path.include?("%") && !path.include?("//") &&
       !path.include?("/.") && !path.start_with?(".") && !path.include?("\\")
      clean = path.empty? || path == "/" ? "/" : (path.end_with?("/") ? path.byteslice(0, path.bytesize - 1) : path)
      return @app.call(env) unless can_serve(clean) || (!clean.equal?(path) && can_serve(path))
    end
    super
  end
end
# config.ru requires rack before main.rb; a tree that loads main.rb first
# (the test helpers) gets Rack::Static here, when the rack gem is there.
begin
  require "rack/static"
rescue LoadError
  nil
end
Rack::Static.prepend(RackStaticPlainMiss) if defined?(Rack::Static)
