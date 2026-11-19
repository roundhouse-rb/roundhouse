#!/usr/bin/env ruby
# The run oracle for native Sidekiq jobs (Phase 5a): what a real Sidekiq
# 8.1.7 server does with a native job's arguments, and with a class's
# `sidekiq_retry_in` and `sidekiq_retries_exhausted` blocks.
#
#   ruby tests/sidekiq_native/run_oracle.rb > tests/sidekiq_native/sidekiq_run.json
#
# Needs sidekiq 8.1.7 and sidekiq-bulk 0.2.0 installed, and redis-server
# and redis-cli on PATH. It starts its own redis-server on a private port
# and one real `sidekiq` server (`-q default -c 1`, poll_interval_average
# 1) with tests/sidekiq_native/oracle_jobs.rb loaded through `-r`. Retry
# entries are rescored to now once their score has been checked against
# the delay the block chose, so the run takes well under a minute.
#
# Scenarios, run one after another on that server:
#   args              - perform(id, name, opts = {}, list = [], flag = nil)
#                       called with 2 and with 5 arguments; jid is set
#   retry_in          - retry: 2, block (count**4) + 15 + count * 10
#   retry_in_kill     - the block returns :kill on the first failure
#   retry_in_discard  - the block returns :discard
#   retry_in_nil      - the block returns nil: Sidekiq's default delay
#   exhausted         - retry: 1, an exhausted block that reads msg["args"]
#   exhausted_not_dead- retry: 0, dead: false, an exhausted block
require "json"
require "rbconfig"
require "tmpdir"
require_relative "../sidekiq_worker/oracle_support"

PORT = 16543
URL = "redis://127.0.0.1:#{PORT}/0"
DIR = File.expand_path(__dir__)
WORK = Dir.mktmpdir("sidekiq-native-run-oracle")
LOG = File.join(WORK, "server.log")
ENV["ORACLE_LOG"] = File.join(WORK, "client.log")
require_relative "oracle_jobs"
Sidekiq.configure_client do |config|
  config.redis = { url: URL }
  config.logger.level = Logger::WARN
end

system("redis-server", "--port", PORT.to_s, "--save", "", "--appendonly", "no",
  "--daemonize", "yes", "--pidfile", "/tmp/sidekiq-native-run-oracle.pid",
  "--logfile", "/tmp/sidekiq-native-run-oracle.log", exception: true)
50.times do
  break if system("redis-cli", "-p", PORT.to_s, "ping", out: File::NULL, err: File::NULL)
  sleep 0.1
end
at_exit { system("redis-cli", "-p", PORT.to_s, "shutdown", "nosave", out: File::NULL, err: File::NULL) }

def redis(*argv) = Sidekiq.redis { |c| c.call(*argv) }

def wait_for(what, seconds = 60)
  deadline = Time.now + seconds
  until yield
    raise "timed out waiting for #{what}" if Time.now > deadline
    sleep 0.05
  end
end

# The delay each scenario's block chooses for a retry count; nil when
# the block isn't consulted or falls back to count**4 + 15.
BLOCK_DELAY = {
  "RetryInWorker" => ->(count) { (count**4) + 15 + (count * 10) }
}.freeze

$checks = []
rescorer = Thread.new do
  loop do
    redis("ZRANGE", "retry", 0, -1, "WITHSCORES").each do |raw, score|
      now = Time.now.to_f
      next unless score > now
      env = JSON.parse(raw)
      count = env["retry_count"]
      at = (env["retried_at"] || env["failed_at"]) / 1000.0
      base = (BLOCK_DELAY[env["class"]] || ->(c) { (c**4) + 15 }).call(count)
      jitter = score - at - base
      ok = jitter > -1 && jitter < 10 * (count + 1) + 1
      raise "#{env["class"]} retry #{count}: score is at + #{(score - at).round(2)}, expected #{base} + rand(#{10 * (count + 1)})" unless ok
      $checks << "#{env["class"]} retry_count #{count}: score = failed + #{base} + rand(#{10 * (count + 1)})"
      redis("ZADD", "retry", "XX", (now - 1).to_s, raw)
    end
    sleep 0.05
  end
end
rescorer.abort_on_exception = true

redis("FLUSHDB")
config = File.join(WORK, "sidekiq.yml")
File.write(config, "poll_interval_average: 1\n")
bin = Gem.bin_path("sidekiq", "sidekiq", "8.1.7")
out = File.join(WORK, "server.out")
PID = Process.spawn({ "ORACLE_LOG" => LOG, "REDIS_URL" => URL },
  RbConfig.ruby, bin, "-r", File.join(DIR, "oracle_jobs.rb"), "-C", config,
  "-q", "default", "-c", "1", "-t", "1", out: out, err: out)
at_exit { Process.kill("KILL", PID) rescue nil }

def log_lines = File.exist?(LOG) ? File.readlines(LOG).size : 0
def trips_since(n) = File.readlines(LOG).drop(n).map { |l| JSON.parse(l) }
def ran = redis("LRANGE", "oracle:ran", 0, -1)

wait_for("the server to boot") { log_lines > 0 }

def mask_native(raw)
  env = JSON.parse(raw)
  raise "jid shape" unless env["jid"].match?(/\A\h{24}\z/)
  env["jid"] = "<jid>"
  %w[created_at enqueued_at failed_at retried_at discarded_at].each do |k|
    next unless env.key?(k)
    raise "#{k} not fresh ms" unless fresh_ms?(env[k])
    env[k] = "<ms>"
  end
  env["error_backtrace"] = "<compressed backtrace>" if env.key?("error_backtrace")
  env
end

# The writes a failure made: retry and dead records, with keys in order.
def writes(trips)
  trips.flat_map do |t|
    t["commands"].filter_map do |argv|
      next unless argv[0].casecmp?("zadd") && %w[retry dead].include?(argv[1])
      { "set" => argv[1], "keys" => top_level_keys(argv[3]), "record" => mask_native(argv[3]) }
    end
  end
end

SCENARIOS = []
def scenario(name)
  redis("DEL", "oracle:ran")
  start = log_lines
  yield
  sleep 1.5 # a poll and a fetch after the last expected event
  trips = trips_since(start)
  SCENARIOS << { "name" => name, "ran" => ran, "writes" => writes(trips),
    "retry_left" => redis("ZCARD", "retry"), "dead_now" => redis("ZCARD", "dead") }
  redis("DEL", "retry", "dead")
end

scenario("args") do
  ArgsWorker.perform_async(1, "n")
  ArgsWorker.perform_async(2, "n", { "k" => "v", "n" => [1, 2] }, [1, 2], true)
  wait_for("args") { ran.size == 2 }
end
scenario("retry_in") do
  RetryInWorker.perform_async(1)
  wait_for("retry_in dead") { redis("ZCARD", "dead") == 1 }
end
scenario("retry_in_kill") do
  RetryInKillWorker.perform_async(1)
  wait_for("retry_in_kill dead") { redis("ZCARD", "dead") == 1 }
end
scenario("retry_in_discard") do
  RetryInDiscardWorker.perform_async(1)
  wait_for("retry_in_discard to fail") { trips_since(0).any? { |t| t["commands"].any? { |c| c[0].casecmp?("brpop") } && t["reply"].to_s.include?("RetryInDiscardWorker") } }
end
scenario("retry_in_nil") do
  RetryInNilWorker.perform_async(1)
  wait_for("retry_in_nil dead") { redis("ZCARD", "dead") == 1 }
end
scenario("exhausted") do
  ExhaustedWorker.perform_async(42)
  wait_for("exhausted dead") { redis("ZCARD", "dead") == 1 && ran.size == 1 }
end
scenario("exhausted_not_dead") do
  ExhaustedNotDeadWorker.perform_async(43)
  wait_for("exhausted_not_dead block") { ran.size == 1 }
end

Process.kill("TERM", PID)
Process.wait(PID)

puts JSON.pretty_generate(
  "versions" => { "sidekiq" => Sidekiq::VERSION, "redis_server" => redis("INFO", "server")[/redis_version:(\S+)/, 1] },
  "retry_score_checks" => $checks.uniq,
  "scenarios" => SCENARIOS
)
