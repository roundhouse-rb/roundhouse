#!/usr/bin/env ruby
# The push oracle for native Sidekiq jobs (Phase 5a): what real Sidekiq
# 8.1.7's client sends to Redis for each way an app enqueues a class that
# includes Sidekiq::Job or Sidekiq::Worker.
#
#   ruby tests/sidekiq_native/oracle.rb > tests/sidekiq_native/sidekiq.json
#
# Needs sidekiq 8.1.7 (with redis-client and connection_pool) and
# sidekiq-bulk 0.2.0 installed, and redis-server and redis-cli on PATH.
# It starts its own redis-server on a private port. No server process is
# needed: every push happens in this process, through the recorder in
# oracle_jobs.rb.
#
# Each scenario is one call. Its record keeps every round trip the call
# made, with the per-run values (jid, created_at, enqueued_at, the
# schedule score) checked for shape and masked, the top-level keys of
# each envelope in written order, and the envelope with those values
# pinned, so a driver can compare bytes.
require "json"
require "tmpdir"
require_relative "../sidekiq_worker/oracle_support"

PORT = 16542
URL = "redis://127.0.0.1:#{PORT}/0"
WORK = Dir.mktmpdir("sidekiq-native-oracle")
LOG = File.join(WORK, "client.log")
ENV["ORACLE_LOG"] = LOG
require_relative "oracle_jobs"
Sidekiq.configure_client do |config|
  config.redis = { url: URL }
  config.logger.level = Logger::WARN
end

system("redis-server", "--port", PORT.to_s, "--save", "", "--appendonly", "no",
  "--daemonize", "yes", "--pidfile", "/tmp/sidekiq-native-oracle.pid",
  "--logfile", "/tmp/sidekiq-native-oracle.log", exception: true)
50.times do
  break if system("redis-cli", "-p", PORT.to_s, "ping", out: File::NULL, err: File::NULL)
  sleep 0.1
end
at_exit { system("redis-cli", "-p", PORT.to_s, "shutdown", "nosave", out: File::NULL, err: File::NULL) }

def redis(*argv) = Sidekiq.redis { |c| c.call(*argv) }

def log_lines = File.exist?(LOG) ? File.readlines(LOG) : []

def native_mask(raw, now)
  env = JSON.parse(raw)
  raise "jid shape: #{env["jid"]}" unless env["jid"].match?(/\A[0-9a-f]{24}\z/)
  env["jid"] = "<jid>"
  %w[created_at enqueued_at].each do |k|
    next unless env.key?(k)
    raise "#{k} is not fresh integer ms: #{env[k].inspect}" unless fresh_ms?(env[k])
    env[k] = "<ms>"
  end
  env
end

# The envelope text with jid, created_at and enqueued_at pinned and
# nothing else touched: keys, order and spelling as Sidekiq wrote them.
def native_pin(raw)
  raw.sub(/"jid":"\h{24}"/, %("jid":"#{PINNED["jid"]}"))
     .sub(/"created_at":\d{13}/, %("created_at":#{PINNED["created_at"]}))
     .sub(/"enqueued_at":\d{13}/, %("enqueued_at":#{PINNED["enqueued_at"]}))
end

def describe(argv, called_at)
  verb = argv[0].downcase
  case verb
  when "lpush"
    envs = argv[2..].map { |raw| JSON.parse(raw) }
    entry = { "argv" => [verb, argv[1]], "envelopes" => argv[2..].map { |raw| envelope(raw) } }
    if envs.size > 1
      entry["one_created_at_per_batch"] = envs.map { |e| e["created_at"] }.uniq.size == 1
      entry["one_enqueued_at_per_batch"] = envs.map { |e| e["enqueued_at"] }.uniq.size == 1
      entry["jids_distinct"] = envs.map { |e| e["jid"] }.uniq.size == envs.size
    end
    entry
  when "zadd"
    score = argv[2]
    raise "score #{score} is not Float#to_s" unless score == score.to_f.to_s
    { "argv" => [verb, argv[1], "<at as Float#to_s>"],
      "at_minus_call_time_s" => (score.to_f - called_at).round,
      "envelopes" => [envelope(argv[3])] }
  else
    { "argv" => [verb] + argv[1..] }
  end
end

def envelope(raw)
  { "keys" => top_level_keys(raw), "masked" => native_mask(raw, nil), "pinned" => native_pin(raw) }
end

def scenario(name, call)
  redis("FLUSHDB")
  before = log_lines.size
  called_at = Time.now.to_f
  result = begin
    value = yield
    { "returned" => value.is_a?(Array) ? "Array of #{value.size} jids" : (value.is_a?(String) && value.match?(/\A\h{24}\z/) ? "<jid>" : value) }
  rescue => e
    { "raised" => "#{e.class}: #{e.message}" }
  end
  trips = log_lines.drop(before).map { |l| JSON.parse(l) }
    .reject { |t| t["commands"].all? { |c| c[0].casecmp?("flushdb") } }
  {
    "name" => name,
    "call" => call,
    **result,
    "round_trips" => trips.map do |t|
      { "kind" => t["kind"], "commands" => t["commands"].map { |argv| describe(argv, called_at) } }
    end
  }
end

FUTURE = 2_000_000_000 # 2033-05-18, an epoch: perform_at / perform_in above 1e9
ARGS = [1, "x", { "a" => "b" }, [2, 3], nil, 1.5, true, false, "é\"\\\n"]

versions = {
  "sidekiq" => Sidekiq::VERSION,
  "sidekiq_bulk" => Gem.loaded_specs["sidekiq-bulk"].version.to_s,
  "redis_client" => RedisClient::VERSION,
  "redis_server" => redis("INFO", "server")[/redis_version:(\S+)/, 1]
}

S = []
# --- perform_async ------------------------------------------------------
S << scenario("async_plain", "PlainWorker.perform_async(1)") { PlainWorker.perform_async(1) }
S << scenario("async_options", 'OkWorker.perform_async(1, {"a" => "b"})') { OkWorker.perform_async(1, { "a" => "b" }) }
S << scenario("async_all_types", "PlainWorker.perform_async(#{ARGS.inspect[1..-2]})") { PlainWorker.perform_async(*ARGS) }
S << scenario("async_no_args", "PlainWorker.perform_async") { PlainWorker.perform_async }
S << scenario("async_all_options", "OptionsWorker.perform_async(7)") { OptionsWorker.perform_async(7) }
S << scenario("async_inherited", "ChildWorker.perform_async(8)") { ChildWorker.perform_async(8) }
S << scenario("async_retry_false", "NoRetryWorker.perform_async(9)") { NoRetryWorker.perform_async(9) }
# --- perform_in / perform_at ---------------------------------------------
S << scenario("in_interval", "OkWorker.perform_in(300, 2)") { OkWorker.perform_in(300, 2) }
S << scenario("in_float", "PlainWorker.perform_in(1.5, 2)") { PlainWorker.perform_in(1.5, 2) }
S << scenario("in_epoch", "OkWorker.perform_in(#{FUTURE}, 2)") { OkWorker.perform_in(FUTURE, 2) }
S << scenario("at_epoch", "OkWorker.perform_at(#{FUTURE}, 3)") { OkWorker.perform_at(FUTURE, 3) }
S << scenario("at_time", "OkWorker.perform_at(Time.at(#{FUTURE}), 3)") { OkWorker.perform_at(Time.at(FUTURE), 3) }
S << scenario("in_zero", "OkWorker.perform_in(0, 4)") { OkWorker.perform_in(0, 4) }
S << scenario("at_past", "OkWorker.perform_at(Time.now - 60, 4)") { OkWorker.perform_at(Time.now - 60, 4) }
S << scenario("in_all_options", "OptionsWorker.perform_in(60, 5)") { OptionsWorker.perform_in(60, 5) }
# --- set ------------------------------------------------------------------
S << scenario("set_queue_async", 'OkWorker.set(queue: "other").perform_async(1)') { OkWorker.set(queue: "other").perform_async(1) }
S << scenario("set_queue_sym_async", "PlainWorker.set(queue: :other).perform_async(1)") { PlainWorker.set(queue: :other).perform_async(1) }
S << scenario("set_queue_in", 'OkWorker.set(queue: "other").perform_in(60, 1)') { OkWorker.set(queue: "other").perform_in(60, 1) }
S << scenario("set_queue_at", "OkWorker.set(queue: \"other\").perform_at(#{FUTURE}, 1)") { OkWorker.set(queue: "other").perform_at(FUTURE, 1) }
S << scenario("set_wait_async", "OkWorker.set(wait: 60).perform_async(1)") { OkWorker.set(wait: 60).perform_async(1) }
S << scenario("set_queue_wait_async", 'OkWorker.set(queue: "other", wait: 60).perform_async(1)') { OkWorker.set(queue: "other", wait: 60).perform_async(1) }
S << scenario("set_wait_until_async", "OkWorker.set(wait_until: #{FUTURE}).perform_async(1)") { OkWorker.set(wait_until: FUTURE).perform_async(1) }
S << scenario("set_retry_async", "OkWorker.set(retry: false).perform_async(1)") { OkWorker.set(retry: false).perform_async(1) }
S << scenario("set_queue_all_options", 'OptionsWorker.set(queue: "other").perform_async(1)') { OptionsWorker.set(queue: "other").perform_async(1) }
S << scenario("set_chained", 'OkWorker.set(queue: "a").set(queue: "b", wait: 60).perform_async(1)') { OkWorker.set(queue: "a").set(queue: "b", wait: 60).perform_async(1) }
# --- bulk -------------------------------------------------------------------
S << scenario("perform_bulk", "OkWorker.perform_bulk([[1], [2], [3]])") { OkWorker.perform_bulk([[1], [2], [3]]) }
S << scenario("perform_bulk_batches", "OkWorker.perform_bulk([[1], [2], [3], [4], [5]], batch_size: 2)") { OkWorker.perform_bulk([[1], [2], [3], [4], [5]], batch_size: 2) }
S << scenario("perform_bulk_set", 'OkWorker.set(queue: "other").perform_bulk([[1], [2]])') { OkWorker.set(queue: "other").perform_bulk([[1], [2]]) }
S << scenario("push_bulk_block", "OkWorker.push_bulk([1, 2, 3, 4, 5], limit: 2) { |i| [i, \"n\#{i}\"] }") { OkWorker.push_bulk([1, 2, 3, 4, 5], limit: 2) { |i| [i, "n#{i}"] } }
S << scenario("push_bulk_no_block", "PlainWorker.push_bulk([1, 2, 3])") { PlainWorker.push_bulk([1, 2, 3]) }
S << scenario("push_bulk_all_options", "OptionsWorker.push_bulk([1, 2])") { OptionsWorker.push_bulk([1, 2]) }
# --- rejected arguments ------------------------------------------------------
S << scenario("reject_symbol", "PlainWorker.perform_async(:sym)") { PlainWorker.perform_async(:sym) }
S << scenario("reject_symbol_keys", "PlainWorker.perform_async({a: 1})") { PlainWorker.perform_async({ a: 1 }) }
S << scenario("reject_time", "PlainWorker.perform_async(Time.at(0))") { PlainWorker.perform_async(Time.at(0)) }
S << scenario("reject_kwargs_style", "PlainWorker.perform_async(1, key: 2)") { PlainWorker.perform_async(1, key: 2) }

puts JSON.pretty_generate("versions" => versions, "scenarios" => S)
