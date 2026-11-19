# Loaded into the real Sidekiq server by tests/sidekiq_heartbeat/
# oracle.rb (`sidekiq -r`). Records every Redis round trip the server
# makes, with each reply, one JSON line per trip, to $ORACLE_LOG, and
# defines the two jobs the heartbeat oracle runs.
gem "sidekiq", "8.1.7"
require "json"
require "sidekiq"

module HeartbeatOracleLog
  LOCK = Mutex.new
  FILE = File.open(ENV.fetch("ORACLE_LOG"), "a")
  FILE.sync = true

  def self.write(entry)
    LOCK.synchronize { FILE.puts(JSON.generate(entry)) }
  end
end

module HeartbeatRecorder
  def call(command, redis_config)
    argv = command.map(&:to_s)
    reply = super
    HeartbeatOracleLog.write("t" => Time.now.to_f, "thread" => Thread.current.name.to_s,
      "kind" => "call", "commands" => [argv], "reply" => reply.is_a?(Float) ? reply.to_s : reply)
    reply
  end

  def call_pipelined(commands, redis_config)
    kind = commands.first&.first.to_s.casecmp?("multi") ? "multi" : "pipelined"
    reply = super
    HeartbeatOracleLog.write("t" => Time.now.to_f, "thread" => Thread.current.name.to_s,
      "kind" => kind, "commands" => commands.map { |c| c.map(&:to_s) }, "reply" => reply)
    reply
  end
end
RedisClient.register(HeartbeatRecorder)

class QuickWorker
  include Sidekiq::Job
  sidekiq_options retry: false
  def perform(n)
    raise "boom #{n}" if n.odd?
  end
end

class NapWorker
  include Sidekiq::Job
  sidekiq_options retry: false
  def perform(seconds)
    Sidekiq.redis { |c| c.call("RPUSH", "oracle:ran", "nap started") }
    sleep seconds
  end
end
