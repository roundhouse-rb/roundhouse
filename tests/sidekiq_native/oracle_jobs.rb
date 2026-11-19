# The native Sidekiq workers the Phase 5 oracles push and run, and the
# recorder that logs every Redis round trip as one JSON line to
# $ORACLE_LOG. Loaded by tests/sidekiq_native/oracle.rb (in process,
# client side) and by run_oracle.rb (into the real server, `sidekiq -r`).
gem "sidekiq", "8.1.7"
require "json"
require "sidekiq"
require "sidekiq/bulk"

module NativeOracleLog
  LOCK = Mutex.new
  FILE = File.open(ENV.fetch("ORACLE_LOG"), "a")
  FILE.sync = true

  def self.write(entry)
    LOCK.synchronize { FILE.puts(JSON.generate(entry)) }
  end
end

module NativeRecorder
  def call(command, redis_config)
    argv = command.map(&:to_s)
    reply = super
    NativeOracleLog.write("t" => Time.now.to_f, "kind" => "call", "commands" => [argv],
      "reply" => reply.is_a?(Float) ? reply.to_s : reply)
    reply
  end

  def call_pipelined(commands, redis_config)
    kind = commands.first&.first.to_s.casecmp?("multi") ? "multi" : "pipelined"
    reply = super
    NativeOracleLog.write("t" => Time.now.to_f, "kind" => kind,
      "commands" => commands.map { |c| c.map(&:to_s) }, "reply" => reply)
    reply
  end
end
RedisClient.register(NativeRecorder)

# --- push oracle --------------------------------------------------------

# No sidekiq_options: the defaults, retry true and queue default.
class PlainWorker
  include Sidekiq::Job
  def perform(*) = nil
end

# The class the plan's §1.2 recording used.
class OkWorker
  include Sidekiq::Job
  sidekiq_options queue: "pull", retry: 5
  def perform(*) = nil
end

# Every literal key Phase 5a carries, in a non-default order, plus the
# sidekiq-unique-jobs keys (carried, not honored).
class OptionsWorker
  include Sidekiq::Worker
  sidekiq_options dead: false, backtrace: true, queue: "push", retry: 8,
    retry_queue: "low", retry_for: 3600, lock: :until_executed, lock_ttl: 86_400
  def perform(*) = nil
end

# Inherits OptionsWorker's options, overrides two and adds one.
class ChildWorker < OptionsWorker
  sidekiq_options queue: "child", backtrace: 5, tags: ["x"]
end

# retry: false and queue_as-style queue via sidekiq_options only.
class NoRetryWorker
  include Sidekiq::Job
  sidekiq_options retry: false, queue: :default
  def perform(*) = nil
end

# --- run oracle (server side) -------------------------------------------

module RunNotes
  def self.note(name, value)
    Sidekiq.redis { |c| c.call("RPUSH", "oracle:ran", "#{name} #{value}") }
  end
end

class StickyError < StandardError; end

# Mastodon's ExponentialBackoff shape: count**4 + 15 + rand jitter, here
# fixed so the retry score is exact.
class RetryInWorker
  include Sidekiq::Job
  sidekiq_options retry: 2
  sidekiq_retry_in do |count|
    (count**4) + 15 + (count * 10)
  end
  def perform(id)
    RunNotes.note("retry_in", "#{id} jid=#{jid}")
    raise StickyError, "retry_in #{id}"
  end
end

# The block's second parameter, and its :kill and :discard results.
class RetryInKillWorker
  include Sidekiq::Job
  sidekiq_options retry: 5
  sidekiq_retry_in do |count, exception, msg|
    RunNotes.note("retry_in_kill_block", "#{count} #{exception.class} #{msg["args"].inspect}")
    :kill
  end
  def perform(id) = raise(StickyError, "kill #{id}")
end

class RetryInDiscardWorker
  include Sidekiq::Job
  sidekiq_options retry: 5
  sidekiq_retry_in { |count| :discard }
  def perform(id) = raise(StickyError, "discard #{id}")
end

# A nil or non-Integer result falls back to Sidekiq's default delay.
class RetryInNilWorker
  include Sidekiq::Job
  sidekiq_options retry: 1
  sidekiq_retry_in { |count| nil }
  def perform(id) = raise(StickyError, "nil #{id}")
end

# Mastodon's exhausted shape: reads msg['args'].first, does work.
class ExhaustedWorker
  include Sidekiq::Job
  sidekiq_options retry: 1
  sidekiq_retries_exhausted do |msg, exception|
    RunNotes.note("exhausted", "#{msg["args"].first} #{exception.class}: #{exception.message} " \
      "retry_count=#{msg["retry_count"]} keys=#{msg.keys.join(",")}")
  end
  def perform(id) = raise(StickyError, "exhausted #{id}")
end

# dead: false with an exhausted block: the block still runs, no dead write.
class ExhaustedNotDeadWorker
  include Sidekiq::Job
  sidekiq_options retry: 0, dead: false
  sidekiq_retries_exhausted do |msg, _ex|
    RunNotes.note("exhausted_not_dead", msg["args"].first.to_s)
  end
  def perform(id) = raise(StickyError, "not dead #{id}")
end

# A job that reads jid and its arguments, including optional ones.
class ArgsWorker
  include Sidekiq::Job
  def perform(id, name, opts = {}, list = [], flag = nil)
    RunNotes.note("args", JSON.generate([id, name, opts, list, flag, jid.length]))
  end
end

# Sleeps across two heartbeats, for the heartbeat oracle.
class SlowWorker
  include Sidekiq::Job
  def perform(seconds)
    RunNotes.note("slow", "started")
    sleep seconds
    RunNotes.note("slow", "done")
  end
end
