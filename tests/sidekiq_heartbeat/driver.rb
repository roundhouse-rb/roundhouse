# The heartbeat (`SidekiqHeartbeat` in runtime/spinel/sidekiq/
# sidekiq_worker.rb) and the poller's process cleanup, over the real
# runtime/spinel/sidekiq/sidekiq_redis.rb and this directory's
# recording Redis (redis.rb), with a scripted clock and identity.
#
# tests/sidekiq_heartbeat.rs copies this file beside the runtime sources
# and runs it under CRuby, with this directory first on the load path,
# and compiled by Spinel. It plays the oracle's timeline
# (tests/sidekiq_heartbeat/sidekiq.json, a real Sidekiq 8.1.7 run with
# `-q default -q low,2 -c 2 -t 3 -g oracle`): an idle beat, the
# poller's cleanup, two beats with a job running and the counts from
# four jobs (two failed) before it, a beat after it finished, the Web
# UI's TTIN, TSTP and TERM arriving through three beats, and the
# shutdown write. It prints one `ok`/`FAIL` line per check, one `trip`
# line per round trip of the timeline, then `done`.
require "json"
require "securerandom"
require "time"
require_relative "base64"

module Rails
  class App
    def global_id_app
      "depot"
    end

    def secret_key_base
      ""
    end
  end

  def self.application
    App.new
  end
end

module ActionController
  class MessageVerifier
    def self.verified_data_json(secret, salt, data, purpose, rotate)
      ""
    end

    def self.json_value(json)
      ""
    end
  end
end

module Db
  def self.with_connection
    result = yield
    result
  end
end

module Broadcasts
  def self.reset_log!
    nil
  end
end

require_relative "active_job"
require_relative "global_id_locator"
require_relative "active_job_serialization"
require_relative "sidekiq_redis"
require_relative "sidekiq_push"
require_relative "job_registry"
require_relative "sidekiq_envelope"
require_relative "sidekiq_worker"

module JobRegistry
  def self.sidekiq_redis_url
    "redis://127.0.0.1:6379/0"
  end

  def self.perform(job_class, args)
    false
  end

  def self.discards_deserialization_error(job_class)
    false
  end
end

def check(name, ok, detail)
  puts (ok ? "ok   " : "FAIL ") + name + (ok ? "" : " (" + detail + ")")
end

def base36?(s)
  return false if s.length == 0
  i = 0
  while i < s.length
    c = s[i]
    return false unless (c >= "0" && c <= "9") || (c >= "a" && c <= "z")
    i += 1
  end
  true
end

def trips_in(section)
  out = []
  Redis::TRIPS.each { |t| out.push(t) if t.start_with?(section + "\t") }
  out
end

SidekiqWorker::LOGGING[0] = false
Redis::TRIPS.clear
Redis::SIGNALS.clear
Redis::MEMBERS.clear
Redis::LIVE.clear

# ---- options the info reports ------------------------------------------------
check("-g sets the tag", SidekiqWorker.parse(["sidekiq", "-g", "web"]) == "" && SidekiqWorker::TAG[0] == "web", SidekiqWorker::TAG[0])
check("the default tag is the directory's name", SidekiqWorker.default_tag("/srv/blog") == "blog", SidekiqWorker.default_tag("/srv/blog"))
check("a Capistrano release is tagged with the app's name",
  SidekiqWorker.default_tag("/srv/blog/releases/20261108120000") == "blog", SidekiqWorker.default_tag("/srv/blog/releases/20261108120000"))
check("no weights: strict", SidekiqWorker.parse(["sidekiq", "-q", "a", "-q", "b"]) == "" && SidekiqWorker.mode == "strict", SidekiqWorker.mode)
check("every weight 1: random", SidekiqWorker.parse(["sidekiq", "-q", "a,1", "-q", "b,1"]) == "" && SidekiqWorker.mode == "random", SidekiqWorker.mode)
check("other weights: weighted", SidekiqWorker.parse(["sidekiq", "-q", "a,3", "-q", "b"]) == "" && SidekiqWorker.mode == "weighted", SidekiqWorker.mode)
SidekiqWorker.parse(["sidekiq", "-q", "a,2", "-q", "b", "-q", "a,3"])
names = SidekiqWorker::QUEUE_NAMES.join(",")
weights = SidekiqWorker::QUEUE_WEIGHTS.map { |w| w.to_s }.join(",")
check("a repeated queue is listed once, with its last weight", names == "a,b" && weights == "3,0", names + " " + weights)

# ---- the oracle's process ----------------------------------------------------
check("the oracle's options", SidekiqWorker.parse(["sidekiq", "-q", "default", "-q", "low,2", "-c", "2", "-t", "3", "-g", "oracle"]) == "", "")
# Two processors, stopped at once: their thread ids, and the state the
# beats read.
SidekiqWorker.start(2)
SidekiqWorker::QUIET[0] = true
waited = 0
while SidekiqWorker.running > 0 && waited < 400
  sleep 0.01
  waited += 1
end
check("the processors stopped", SidekiqWorker.running == 0, SidekiqWorker.running.to_s)
SidekiqWorker::QUIET[0] = false
tids = SidekiqWorker::TIDS
check("each processor has its own base-36 tid",
  tids.length == 2 && tids[0] != tids[1] && base36?(tids[0]) && base36?(tids[1]), tids.join(","))
tids[0] = (0 ^ 4242).to_s(36)
tids[1] = (1 ^ 4242).to_s(36)

# The counts a beat flushes: a job that ran and failed is both; text
# that isn't a job is neither, as in Sidekiq.
SidekiqWorker.take_counts
SidekiqWorker.run("default", "{\"class\":\"QuickWorker\",\"args\":[1],\"retry\":false,\"queue\":\"default\",\"jid\":\"j1\"}")
SidekiqWorker.run("default", "{")
SidekiqWorker.run("default", "null")
counts = SidekiqWorker.take_counts
check("a failed job counts as processed and failed, a malformed one not at all",
  counts[0] == 1 && counts[1] == 1, counts[0].to_s + " " + counts[1].to_s)
check("the counts start again from 0", SidekiqWorker.take_counts[0] == 0, "")

identity = "host-1:4242:0123456789ab"
SidekiqHeartbeat.setup("host-1", 4242, "0123456789ab", 1792632700.25)
check("identity", SidekiqHeartbeat.identity == identity, SidekiqHeartbeat.identity)
Redis::SIGNALS.push("")
Redis::SIGNALS.push("")
Redis::SIGNALS.push("")
Redis::SIGNALS.push("TTIN")
Redis::SIGNALS.push("TSTP")
Redis::SIGNALS.push("")
Redis::SIGNALS.push("TERM")
Redis::MEMBERS.push(identity)
Redis::LIVE.push(identity)
NAP = "{\"retry\":false,\"queue\":\"default\",\"args\":[25],\"class\":\"NapWorker\",\"jid\":\"4f2a9c1e0b7d3a6f5c8e1d20\",\"created_at\":1792632740123,\"enqueued_at\":1792632740124}"
T = 1792632731.5

def beat(at)
  SidekiqWorker.with_control { |conn| SidekiqHeartbeat.beat(conn, at) }
end

# The control connection is open before the timeline, as it would be.
SidekiqWorker.with_control { |conn| nil }
Redis::TRIPS.clear

Redis::SECTION[0] = "heartbeat"
beat(T)
Redis::SECTION[0] = "cleanup"
dropped = [0]
SidekiqWorker.with_control { |conn| dropped[0] = SidekiqRedis.cleanup_processes(conn) }
check("the cleanup keeps a process that beats", dropped[0] == 0, dropped[0].to_s)

Redis::SECTION[0] = "heartbeat"
SidekiqWorker::LOCK.synchronize do
  SidekiqWorker::PROCESSED[0] = 4
  SidekiqWorker::FAILED[0] = 2
  SidekiqWorker::INFLIGHT_QUEUE[1] = "queue:default"
  SidekiqWorker::INFLIGHT_JOB[1] = NAP
  SidekiqWorker::INFLIGHT_RUN_AT[1] = 1792632741
end
beat(T + 10.0)
beat(T + 20.0)
SidekiqWorker::LOCK.synchronize do
  SidekiqWorker::INFLIGHT_QUEUE[1] = ""
  SidekiqWorker::INFLIGHT_JOB[1] = ""
  SidekiqWorker::PROCESSED[0] = 1
end
beat(T + 30.0)
check("TTIN from the Web UI sets the thread-dump flag", SidekiqWorker::SIGNALS[2] == 1, SidekiqWorker::SIGNALS[2].to_s)
beat(T + 40.0)
check("TSTP from the Web UI sets the quiet flag", SidekiqWorker::SIGNALS[1] == 1, SidekiqWorker::SIGNALS[1].to_s)
# What the supervisor does with it.
SidekiqWorker::QUIET[0] = true
beat(T + 50.0)
beat(T + 60.0)
check("TERM from the Web UI sets the stop flag", SidekiqWorker::SIGNALS[0] == 1, SidekiqWorker::SIGNALS[0].to_s)
Redis::SECTION[0] = "shutdown"
SidekiqHeartbeat.clear
timeline = Redis::TRIPS.dup

# ---- beyond the oracle -------------------------------------------------------
Redis::TRIPS.clear
Redis::SECTION[0] = "dead"
Redis::MEMBERS.clear
Redis::MEMBERS.push(identity)
Redis::MEMBERS.push("old-host:1:aaaaaaaaaaaa")
SidekiqWorker.with_control { |conn| dropped[0] = SidekiqRedis.cleanup_processes(conn) }
dead = trips_in("dead")
check("the cleanup drops a process whose hash expired, and only it",
  dropped[0] == 1 && dead.length == 5 && dead[4].include?("[\"SREM\",\"processes\",\"old-host:1:aaaaaaaaaaaa\"]"), dead.join(" / "))

Redis::SECTION[0] = "lost"
Redis::CLEANUP_WON[0] = false
SidekiqWorker.with_control { |conn| dropped[0] = SidekiqRedis.cleanup_processes(conn) }
check("another process cleaned up this minute: nothing more", dropped[0] == 0 && trips_in("lost").length == 1, trips_in("lost").join(" / "))

Redis::SECTION[0] = "last"
SidekiqWorker::LOCK.synchronize { SidekiqWorker::PROCESSED[0] = 2 }
SidekiqHeartbeat.clear
last = trips_in("last")
check("the shutdown write flushes the last counts first",
  last.length == 3 && last[0].include?("[\"INCRBY\",\"stat:processed\",\"2\"]") && last[1].include?("SREM"), last.join(" / "))

Redis::SECTION[0] = "unknown"
SidekiqWorker::SIGNALS[0] = 0
SidekiqHeartbeat.act("USR2")
check("a signal it doesn't know sets nothing", SidekiqWorker::SIGNALS[0] == 0, "")

# The supervisor's side of the handoff: it takes TTIN and leaves the rest set.
SidekiqWorker::SIGNALS[0] = 0
SidekiqWorker::SIGNALS[1] = 0
SidekiqWorker::SIGNALS[2] = 0
SidekiqHeartbeat.act("TTIN")
SidekiqHeartbeat.act("TSTP")
seen = SidekiqWorker.take_signals
check("the supervisor takes TTIN and keeps TSTP",
  seen[0] == 0 && seen[1] == 1 && seen[2] == 1 && SidekiqWorker::SIGNALS[1] == 1 && SidekiqWorker::SIGNALS[2] == 0, seen.join(","))
SidekiqWorker::SIGNALS[1] = 0

# Where there is no /proc, the resident set comes from `ps`.
check("ps reads this process's resident set", SidekiqHeartbeat.ps_rss_kb(Process.pid) > 0, SidekiqHeartbeat.ps_rss_kb(Process.pid).to_s)
check("ps answers 0 for a pid it doesn't list", SidekiqHeartbeat.ps_rss_kb(999999999) == 0, SidekiqHeartbeat.ps_rss_kb(999999999).to_s)

timeline.each { |t| puts "trip\t" + t }
puts "done"
