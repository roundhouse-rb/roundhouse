#!/usr/bin/env ruby
# The heartbeat oracle (Phase 5b): what a real Sidekiq 8.1.7 process
# writes to Redis to be visible to the Web UI, while idle, while busy,
# and when the Web UI's buttons signal it.
#
#   ruby tests/sidekiq_heartbeat/oracle.rb > tests/sidekiq_heartbeat/sidekiq.json
#
# Needs sidekiq 8.1.7 installed and redis-server and redis-cli on PATH.
# It starts its own redis-server on a private port and one real
# `sidekiq` server (`-q default -q low,2 -c 2 -g oracle`, poll_interval_average 2)
# with tests/sidekiq_heartbeat/oracle_jobs.rb loaded through `-r`.
#
# Timeline (the beat is every 10 seconds and can't be changed):
#   boot, first beat (idle)
#   4 quick jobs (2 fail) and one NapWorker sleeping 25 s
#   two beats with the nap running: <identity>:work has content
#   Sidekiq::Process#dump_threads (TTIN), one beat
#   Sidekiq::Process#quiet! (TSTP), one beat: quiet "true"
#   Sidekiq::Process#stop!  (TERM), shutdown: SREM and UNLINK
#
# Only heartbeat, stats, signal, cleanup and shutdown commands are kept;
# fetches and the jobs' own bookkeeping are dropped. Values that change
# per run (hostname, pid, nonce, times, RSS, RTT, dates, jids) are
# checked for shape and masked.
require "json"
require "rbconfig"
require "socket"
require "tmpdir"
require_relative "../sidekiq_worker/oracle_support"

PORT = 16544
URL = "redis://127.0.0.1:#{PORT}/0"
DIR = File.expand_path(__dir__)
WORK = Dir.mktmpdir("sidekiq-heartbeat-oracle")
LOG = File.join(WORK, "server.log")
ENV["ORACLE_LOG"] = File.join(WORK, "client.log")
require_relative "oracle_jobs"
require "sidekiq/api"
Sidekiq.configure_client do |config|
  config.redis = { url: URL }
  config.logger.level = Logger::WARN
end

system("redis-server", "--port", PORT.to_s, "--save", "", "--appendonly", "no",
  "--daemonize", "yes", "--pidfile", "/tmp/sidekiq-heartbeat-oracle.pid",
  "--logfile", "/tmp/sidekiq-heartbeat-oracle.log", exception: true)
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

redis("FLUSHDB")
config = File.join(WORK, "sidekiq.yml")
File.write(config, "poll_interval_average: 2\n")
bin = Gem.bin_path("sidekiq", "sidekiq", "8.1.7")
out = File.join(WORK, "server.out")
PID = Process.spawn({ "ORACLE_LOG" => LOG, "REDIS_URL" => URL },
  RbConfig.ruby, bin, "-r", File.join(DIR, "oracle_jobs.rb"), "-C", config,
  "-q", "default", "-q", "low,2", "-c", "2", "-t", "3", "-g", "oracle", out: out, err: out)
at_exit { Process.kill("KILL", PID) rescue nil }

def trips = File.exist?(LOG) ? File.readlines(LOG).map { |l| JSON.parse(l) } : []
def beats = trips.count { |t| t["kind"] == "multi" && t["commands"][1]&.first&.casecmp?("sadd") }
def proc_entry = Sidekiq::ProcessSet.new.find { |p| p["pid"] == PID }

timeline = []
wait_for("the first beat") { beats >= 1 }
timeline << "beat 1: idle"
4.times { |i| QuickWorker.perform_async(i) }
NapWorker.perform_async(25)
wait_for("the nap to start") { redis("LRANGE", "oracle:ran", 0, -1).include?("nap started") }
b = beats
wait_for("two busy beats", 30) { beats >= b + 2 }
timeline << "beats #{b + 1}-#{b + 2}: busy, ProcessSet busy=#{proc_entry["busy"]}"
work_seen = Sidekiq::WorkSet.new.to_a.map { |_p, _t, w| w.queue + " " + w.payload["class"] }
proc_entry.dump_threads
b = beats
wait_for("the TTIN beat", 30) { beats >= b + 1 }
timeline << "beat #{b + 1}: after dump_threads"
proc_entry.quiet!
b = beats
wait_for("the TSTP beat and the next", 30) { beats >= b + 2 }
timeline << "beats #{b + 1}-#{b + 2}: after quiet!, ProcessSet quiet=#{proc_entry["quiet"]}"
proc_entry.stop!
Process.wait(PID)
timeline << "exited; processes holds #{redis("SCARD", "processes")}"

# --- describe -------------------------------------------------------------
all = trips
identity = all.lazy.flat_map { |t| t["commands"] }.find { |c| c[0].casecmp?("sadd") && c[1] == "processes" }[2]
host, pid, nonce = identity.split(":")
raise "identity shape #{identity}" unless pid.to_i == PID && nonce.match?(/\A\h{12}\z/) && host == (ENV["DYNO"] || Socket.gethostname)

def float_s?(s) = s.is_a?(String) && s.match?(/\A\d+\.\d+(e[+-]?\d+)?\z/) && s.to_f.to_s == s

def mask_text(s, identity)
  s.gsub(identity, "<identity>").gsub(/stat:(processed|failed):\d{4}-\d\d-\d\d/, 'stat:\1:<YYYY-MM-DD>')
end

def mask_info(raw, identity)
  info = JSON.parse(raw)
  host, pid, = identity.split(":")
  raise "info hostname" unless info["hostname"] == host
  raise "info pid" unless info["pid"] == pid.to_i
  raise "info identity" unless info["identity"] == identity
  raise "info started_at" unless info["started_at"].is_a?(Float)
  info.merge("hostname" => "<hostname>", "pid" => "<pid>", "identity" => "<identity>", "started_at" => "<epoch Float>")
end

def mask_work(raw)
  w = JSON.parse(raw)
  raise "run_at #{w["run_at"]}" unless w["run_at"].is_a?(Integer) && w["run_at"].to_s.length == 10
  payload = JSON.parse(w["payload"])
  payload["jid"] = "<jid>"
  %w[created_at enqueued_at].each { |k| payload[k] = "<ms>" if payload.key?(k) }
  { "keys" => w.keys, "queue" => w["queue"], "payload_is_raw_job_text" => w["payload"].is_a?(String),
    "payload" => payload, "run_at" => "<epoch seconds, Integer>" }
end

def describe(argv, identity)
  verb = argv[0].downcase
  a = argv.map { |x| mask_text(x, identity) }
  case verb
  when "hset"
    if a[1] == "<identity>"
      fields = argv[2..].each_slice(2).to_h
      m = fields.to_h do |k, v|
        case k
        when "info" then [k, mask_info(v, identity)]
        when "beat" then raise("beat #{v}") unless float_s?(v); [k, "<epoch Float#to_s>"]
        when "rtt_us" then raise("rtt") unless v.match?(/\A\d+\z/); [k, "<Integer µs>"]
        when "rss" then raise("rss") unless v.match?(/\A\d+\z/); [k, "<Integer kB>"]
        else [k, v]
        end
      end
      { "argv" => ["hset", "<identity>"], "field_order" => fields.keys, "fields" => m }
    else
      tids = argv[2..].each_slice(2).map(&:first)
      raise "tid shape #{tids}" unless tids.all? { |t| t.match?(/\A[0-9a-z]+\z/) }
      { "argv" => ["hset", a[1]], "entries" => argv[2..].each_slice(2).map { |_t, v| { "tid" => "<base-36 tid>", "value" => mask_work(v) } } }
    end
  when "lpush", "rpush"
    { "argv" => a.first(2) + ["<#{argv.size - 2} value(s)>"] }
  else
    { "argv" => a }
  end
end

KEEP = %w[multi exec sadd srem exists hset expire rpop unlink incrby ping set sscan hget scard del lpush hgetall].freeze

def keep?(trip)
  cmds = trip["commands"]
  return false if cmds.any? { |c| c[1].to_s.start_with?("oracle:") }
  return false if cmds.all? { |c| %w[brpop evalsha zrange script].include?(c[0].downcase) }
  return false if cmds.any? { |c| c[0].casecmp?("lpush") && c[1].to_s.start_with?("queue:") }
  cmds.all? { |c| KEEP.include?(c[0].downcase) }
end

described = all.select { |t| keep?(t) }.map do |t|
  entry = { "kind" => t["kind"], "thread" => t["thread"], "commands" => t["commands"].map { |c| describe(c, identity) } }
  entry["reply"] = t["reply"].map { |r| r.is_a?(String) ? mask_text(r, identity) : r } if t["kind"] == "multi"
  entry
end
# Consecutive identical round trips (idle beats) are written once, with a count.
compressed = described.chunk_while { |a, b| a == b }.map { |run| run.first.merge("times" => run.size) }

puts JSON.pretty_generate(
  "versions" => { "sidekiq" => Sidekiq::VERSION, "redis_client" => RedisClient::VERSION,
                  "redis_server" => redis("INFO", "server")[/redis_version:(\S+)/, 1] },
  "args" => "-q default -q low,2 -c 2 -t 3 -g oracle (poll_interval_average 2)",
  "identity_shape" => "<hostname>:<pid>:<12 hex>",
  "timeline" => timeline,
  "work_set_seen_by_api" => work_seen,
  "round_trips" => compressed
)
