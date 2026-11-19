# scripts/lib/minitest/app_suite_plugin.rb — one JSON row per test the
# app's own `bin/rails test` runs, for scripts/app-suite-baseline.
#
# app-suite-baseline puts scripts/lib on RUBYLIB and appends
# `require "minitest/app_suite_plugin"` to the scratch copy's
# test/test_helper.rb. Minitest 6 loads no plugin by itself, so the file
# registers itself; Minitest 5 also finds it on the load path
# (Gem.find_files). It does nothing unless APP_SUITE_JSON names the file
# to write.
#
# Rails' parallel executor records each result in the parent process, so
# one file holds every worker's rows.

require "json"

module Minitest
  def self.plugin_app_suite_init(_options)
    path = ENV["APP_SUITE_JSON"]
    reporter << AppSuiteReporter.new(path) if path && !path.empty?
  end

  class AppSuiteReporter < AbstractReporter
    def initialize(path)
      super()
      @path = path
      @rows = []
    end

    def record(result)
      failure = result.failure
      error = failure.is_a?(UnexpectedError) ? failure.error : failure
      file, line = result.source_location
      @rows << {
        "name" => "#{result.klass}##{result.name}",
        "file" => file.to_s.delete_prefix("#{Dir.pwd}/"),
        "line" => line,
        "status" => status_of(result, failure),
        "exception" => error&.class&.name,
        "message" => error && error.message.to_s.lines.first.to_s.strip[0, 200],
        "location" => failure && failure.location.to_s.delete_prefix("#{Dir.pwd}/"),
      }.compact
    end

    def report
      File.write(@path, JSON.generate(@rows))
    end

    private

    def status_of(result, failure)
      if result.skipped? then "skip"
      elsif result.passed? then "pass"
      elsif failure.is_a?(UnexpectedError) then "error"
      else "fail"
      end
    end
  end
end

Minitest.register_plugin(:app_suite) if Minitest.respond_to?(:register_plugin)
