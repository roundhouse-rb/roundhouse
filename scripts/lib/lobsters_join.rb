# scripts/lib/lobsters_join.rb — the row join between lobsters' own RSpec
# suite run against the emit and Rails' own run of the same suite.
#
# The failure list alone cannot see two things. A spec that fails on stock
# Rails too is the environment, not the compiler — lobsters' suite has
# order-dependent failures — and a spec whose file never loads against the
# emit has no failure to count at all: no example, no exception, just a
# smaller denominator. Only Rails' run names either, so the two are joined
# example by example and each row gets one of five statuses:
#
#   match    Rails passes it, the emit passes it
#   gap      Rails passes it, the emit fails it
#   dropped  Rails runs it, the emit produced no such example
#   env      it fails on Rails too — set aside from the cause counts
#   skip     either side skipped it
#
# A PURE FUNCTION of the two reports, in its own file so
# tests/lobsters_join_test.py can exercise every status from fixtures
# without an emit tree, the shim, a bundle or a Rails boot.
# scripts/lobsters-specs is its only caller.
#
# IDs, NOT DESCRIPTIONS. RSpec reuses one full_description for every
# example in a looped or shared context, so matching on it would collapse
# dozens of examples into one. The id (`spec/models/x_spec.rb[1:2:1]`) is
# unique per example, and the two runs differ only in its path prefix: the
# emit run is handed absolute suite paths, the Rails run runs in a scratch
# copy of the checkout and reports `./spec/...`.
module LobstersJoin
  ROWS = %w[match gap dropped env skip].freeze

  module_function

  # `<file>[<scopes>]`, normalized to `spec/…`. An absolute id is made
  # relative to the checkout `root` when given (a checkout can itself sit
  # under a directory named `spec`); without one, at its first `/spec/`.
  def example_id(id, root = nil)
    s = id.to_s.sub(%r{\A\./}, "")
    return s unless s.start_with?("/")
    prefix = root && "#{File.expand_path(root)}/"
    prefix && s.start_with?(prefix) ? s.delete_prefix(prefix) : s.sub(%r{\A.*?/spec/}, "spec/")
  end

  # emit_examples: the emit run's `examples` (RSpec JSON), whose status is
  # passed / failed / pending. baseline: scripts/app-suite-baseline's
  # report, whose rows carry pass / fail / error / skip.
  #
  # Returns rows (counts by status over the examples RAILS runs),
  # dropped and env (each an example record), and env_ids — the ids the
  # caller must keep out of its cause counts.
  def rows(emit_examples:, baseline:, root: nil)
    emit_by_id = {}
    emit_examples.each { |e| emit_by_id[example_id(e["id"], root)] ||= e }
    counts = Hash.new(0)
    dropped = []
    env = []
    env_ids = {}

    Array(baseline["tests"]).each do |t|
      id = example_id(t["id"], root)
      e = emit_by_id[id]
      rails_status = t["status"].to_s
      status =
        if %w[fail error].include?(rails_status)
          "env"
        elsif rails_status == "skip" || (e && e["status"] == "pending")
          "skip"
        elsif e.nil?
          "dropped"
        elsif e["status"] == "passed"
          "match"
        else
          "gap"
        end
      counts[status] += 1
      env_ids[id] = true if status == "env"
      dropped << { "id" => id, "name" => t["name"], "file" => t["file"] } if status == "dropped"
      if status == "env"
        env << { "id" => id, "name" => t["name"], "file" => t["file"],
                 "exception" => t["exception"], "message" => t["message"] }
      end
    end

    { "rows" => ROWS.to_h { |k| [k, counts[k]] }, "dropped" => dropped,
      "env" => env, "env_ids" => env_ids }
  end
end
