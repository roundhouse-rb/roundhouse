# A recording stand-in for the Redis client that
# runtime/spinel/sidekiq/sidekiq_redis.rb runs over (spinel-redis on
# spinel, redis-rb on ruby and jruby), for
# tests/sidekiq_heartbeat/driver.rb. sidekiq_redis.rb's `require
# "redis"` finds this file: tests/sidekiq_heartbeat.rs runs the driver
# with this directory first on the load path.
#
# Each round trip the runtime makes is kept in TRIPS as its kind
# (`call`, `pipelined` or `multi`) and its commands, written as
# spinel-redis writes them, with the MULTI and EXEC around a
# transaction as the oracle records them. The replies the heartbeat
# reads come from a script: SIGNALS for each beat's RPOP, MEMBERS and
# LIVE for the poller's cleanup. The `rtt_us` and `rss` a beat writes
# change from run to run; each is checked for digits and kept as
# `<int>`, so CRuby and the Spinel binary print the same.
class Redis
  class BaseError < StandardError
  end

  class BaseConnectionError < BaseError
  end

  # "<section>\t<kind>\t<commands as JSON>", one per round trip.
  TRIPS = [""]
  SECTION = ["setup"]
  # What each beat's RPOP answers, in order; "" for nothing.
  SIGNALS = [""]
  # The members SSCAN walks, and those whose heartbeat hash still lives.
  MEMBERS = [""]
  LIVE = [""]
  # What `SET process_cleanup 1 NX EX 60` answers.
  CLEANUP_WON = [true]
  L = Mutex.new

  def initialize(url: "", password: "", connect_timeout: 1.0, read_timeout: 1.0)
    @url = url
  end

  def self.log(kind, commands)
    text = []
    commands.each do |argv|
      quoted = []
      argv.each { |a| quoted.push(ActiveJob::Arguments.str(a)) }
      text.push("[" + quoted.join(",") + "]")
    end
    L.synchronize { TRIPS.push(SECTION[0] + "\t" + kind + "\t[" + text.join(",") + "]") }
    nil
  end

  def self.next_signal
    L.synchronize do
      s = ""
      if SIGNALS.length > 0
        s = SIGNALS[0]
        SIGNALS.delete_at(0)
      end
      s
    end
  end

  def self.digits?(s)
    return false if s.length == 0
    i = 0
    while i < s.length
      c = s[i]
      return false unless c >= "0" && c <= "9"
      i += 1
    end
    true
  end

  def pipelined
    r = RedisRecorder.new
    yield r
    Redis.log("pipelined", r.argvs)
    out = []
    r.argvs.each { |a| out.push("") }
    out
  end

  def multi
    r = RedisRecorder.new
    yield r
    commands = [["MULTI"]]
    r.argvs.each { |a| commands.push(a) }
    commands.push(["EXEC"])
    Redis.log("multi", commands)
    out = []
    r.argvs.each do |a|
      if a[0] == "RPOP"
        s = Redis.next_signal
        out.push(s == "" ? nil : s)
      else
        out.push("")
      end
    end
    out
  end

  def ping
    Redis.log("call", [["PING"]])
    "PONG"
  end

  # spinel-redis's SET: key value [EX n] [NX].
  def set(key, value, ex: 0, nx: false)
    argv = ["SET", key.to_s, value.to_s]
    if ex > 0
      argv.push("EX")
      argv.push(ex.to_s)
    end
    argv.push("NX") if nx
    Redis.log("call", [argv])
    CLEANUP_WON[0]
  end

  def sscan_each(key)
    Redis.log("call", [["SSCAN", key.to_s, "0"]])
    members = MEMBERS.dup
    members.each { |m| yield m }
    nil
  end

  def hget(key, field)
    Redis.log("call", [["HGET", key.to_s, field.to_s]])
    LIVE.include?(key) ? "{}" : nil
  end

  def srem(key, member)
    Redis.log("call", [["SREM", key.to_s, member.to_s]])
    1
  end

  def unlink(key)
    Redis.log("call", [["UNLINK", key.to_s]])
    1
  end

  # The processor threads the driver starts and stops at once.
  def brpop(keys, timeout: 0)
    sleep 0.01
    []
  end

  def close
    nil
  end
end

# The block argument of `pipelined` and `multi`: the commands queued.
class RedisRecorder
  def initialize
    @argvs = [[""]]
    @argvs.clear
  end

  def argvs
    @argvs
  end

  def add(argv)
    @argvs.push(argv)
    nil
  end

  def incrby(key, n)
    add(["INCRBY", key.to_s, n.to_s])
  end

  def expire(key, seconds)
    add(["EXPIRE", key.to_s, seconds.to_s])
  end

  def unlink(key)
    add(["UNLINK", key.to_s])
  end

  # The Hash form, the one the heartbeat uses. A beat's rtt_us and rss
  # are kept as <int> once checked for digits.
  def hset(key, fields)
    argv = ["HSET", key.to_s]
    fields.each do |f, v|
      argv.push(f)
      if (f == "rtt_us" || f == "rss") && Redis.digits?(v)
        argv.push("<int>")
      else
        argv.push(v)
      end
    end
    add(argv)
  end

  def sadd(key, member)
    add(["SADD", key.to_s, member.to_s])
  end

  def exists(key)
    add(["EXISTS", key.to_s])
  end

  def rpop(key)
    add(["RPOP", key.to_s])
  end

  def lpush(key, value)
    add(["LPUSH", key.to_s, value.to_s])
  end

  def rpush(key, values)
    argv = ["RPUSH", key.to_s]
    values.each { |v| argv.push(v) }
    add(argv)
  end

  def zadd(key, score, member)
    add(["ZADD", key.to_s, score.to_s, member.to_s])
  end

  def zremrangebyscore(key, min, max)
    add(["ZREMRANGEBYSCORE", key.to_s, min.to_s, max.to_s])
  end

  def zremrangebyrank(key, start, stop)
    add(["ZREMRANGEBYRANK", key.to_s, start.to_s, stop.to_s])
  end
end
