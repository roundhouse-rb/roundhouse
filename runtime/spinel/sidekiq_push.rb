# frozen_string_literal: true
# Pushing ActiveJob payloads to Redis the way Sidekiq 8.1's ActiveJob
# adapter does, for an app on `config.active_job.queue_adapter =
# :sidekiq`. The same build's worker role (`./app sidekiq`, or
# `ruby sidekiq.rb`; runtime/spinel/sidekiq/sidekiq_worker.rb) runs the
# jobs, or a Ruby Sidekiq process in a Rails app with the same job
# classes and database does.
#
# THE BYTES ARE SIDEKIQ'S. The ActiveJob payload (`Payload.build_at`)
# goes inside Sidekiq's job envelope, with the keys in the order
# Sidekiq writes them:
#
#   {"retry":true,"queue":"default","wrapped":"InspectCarJob",
#    "args":[<payload>],"class":"Sidekiq::ActiveJob::Wrapper",
#    "jid":"<24 hex>","created_at":<ms>,"enqueued_at":<ms>}
#
# An immediate job is `SADD queues <queue>` plus `LPUSH queue:<queue>`
# in one round trip; a scheduled one is `ZADD schedule <epoch> <json>`,
# with no `enqueued_at`. `tests/sidekiq_push.rs` holds this to what real
# Sidekiq 8.1.7 sends (`tests/sidekiq_push/sidekiq.json`).
#
# NOTHING HERE NAMES `Redis`. The connection lives in
# `runtime/sidekiq_redis.rb`, which a tree on `:sidekiq` gets with a
# real client (spinel-redis on spinel, redis-rb on ruby and jruby) and
# every other tree gets as a raising façade (`project::spin_shape` and
# `ruby_family_runtime_files` do the swap). Nothing calls this module
# unless the lowering wrote a `JobRegistry.push_<job>` call, which it
# does only under `:sidekiq`.
#
# The Redis URL is the app's own `Sidekiq.configure_client` `url:`
# (`JobRegistry.sidekiq_redis_url`), and nothing else: ingest refuses a
# tree on `:sidekiq` that gives none, so there is no fallback to
# `REDIS_PROVIDER`, `REDIS_URL` or localhost. A `url:` read from an
# environment variable that is unset or empty at run time raises at the
# first push. A `password:` from the same block overrides one in the
# URL, as redis-rb's option does. Both are read at the first push in
# each process. Errors reach the caller, as in Rails: a down Redis makes
# `perform_later` raise.
module SidekiqPush
  URL = [""]
  URL_PID = [0]

  # The test seam: while recording, pushes are written to RECORDED as
  # text instead of going to Redis, so a test needs no server and no
  # client. Same shape as `SecureRandom.stub_uuid`.
  RECORDING = [false]
  RECORDED = [""]

  # One push held until COMMIT (`enqueue_after_commit`).
  class HeldPush
    def initialize(job_class, queue, at, aj_json)
      @job_class = job_class
      @queue = queue
      @at = at
      @aj_json = aj_json
    end

    def job_class
      @job_class
    end

    def queue
      @queue
    end

    def at
      @at
    end

    def aj_json
      @aj_json
    end
  end

  # One job, given its ActiveJob payload. `at` is the scheduled epoch in
  # seconds, 0.0 when the job isn't scheduled. A time that has already
  # passed is pushed at once, as Sidekiq's `Setter#at` does. The job's
  # `sidekiq_options` go where Sidekiq writes them: `retry` in place,
  # the rest after `queue`.
  def self.enqueue(job_class, queue, at, aj_json)
    head = "{\"retry\":" + JobRegistry.sidekiq_retry(job_class) +
      ",\"queue\":" + ActiveJob::Arguments.str(queue) +
      JobRegistry.sidekiq_extra(job_class) +
      ",\"wrapped\":" + ActiveJob::Arguments.str(job_class) +
      ",\"args\":[" + aj_json + "]" +
      ",\"class\":\"Sidekiq::ActiveJob::Wrapper\"" +
      ",\"jid\":" + ActiveJob::Arguments.str(SecureRandom.hex(12)) +
      ",\"created_at\":" + now_ms.to_s
    if at > Time.now.to_f
      schedule(at, head + "}")
    else
      push(queue, head + ",\"enqueued_at\":" + now_ms.to_s + "}")
    end
    nil
  end

  # One native Sidekiq job (a class that includes `Sidekiq::Job` or
  # `Sidekiq::Worker`), in the envelope Sidekiq 8.1's client writes:
  #
  #   {"retry":5,"queue":"pull",<the class's other options>,
  #    "args":[1,{"a":"b"}],"class":"OkWorker",
  #    "jid":"<24 hex>","created_at":<ms>,"enqueued_at":<ms>}
  #
  # `retry_json` and `extra` are JSON text: the envelope's `retry` and
  # the class's other `sidekiq_options` as `,"key":value` pairs.
  # `args_json` is the argument list.
  #
  # `at` is what the call named, still to go through Sidekiq's rule: a
  # value under 1,000,000,000 is an interval from now, anything larger
  # an epoch, and 0.0 is no delay. A time that isn't in the future is
  # pushed at once, as Sidekiq does. `shape` is how Sidekiq builds the
  # item: 0 through its `Setter`, which writes `args` first; 1 for a bare
  # class-side `perform_in` / `perform_at`, which it builds itself with
  # `class` before `args`, whether or not it ends up scheduled; 2 for one
  # job of a bulk push, held for its batch while one is open. Only a bulk
  # job is held: another push the bulk block makes goes out as it would
  # anywhere else.
  def self.enqueue_native(job_class, retry_json, queue, extra, at, shape, args_json)
    open = bulks
    if shape == 2 && open.length > 0
      batch = open[open.length - 1]
      batch.jobs.push(HeldNative.new(job_class, retry_json, queue, extra, args_json))
      flush_batch(batch.jobs) if batch.jobs.length >= batch.limit
      return nil
    end
    now = Time.now.to_f
    ts = at < 1_000_000_000.0 ? now + at : at
    scheduled = ts > now
    body = if shape != 0
      ",\"class\":" + ActiveJob::Arguments.str(job_class) + ",\"args\":" + args_json
    else
      ",\"args\":" + args_json + ",\"class\":" + ActiveJob::Arguments.str(job_class)
    end
    head = "{\"retry\":" + retry_json +
      ",\"queue\":" + ActiveJob::Arguments.str(queue) +
      extra + body +
      ",\"jid\":" + ActiveJob::Arguments.str(SecureRandom.hex(12)) +
      ",\"created_at\":" + now_ms.to_s
    if scheduled
      schedule(ts, head + "}")
    else
      push(queue, head + ",\"enqueued_at\":" + now_ms.to_s + "}")
    end
    nil
  end

  # A native job's argument whose JSON kind is known only now: a call
  # site passes a String here and a Boolean there, or a Hash's values
  # are of different kinds. Written as Sidekiq's `JSON.generate` writes
  # each kind.
  def self.native_value(value)
    if value.nil?
      "null"
    elsif value == true
      "true"
    elsif value == false
      "false"
    elsif value.is_a?(Integer)
      value.to_s
    elsif value.is_a?(Float)
      ActiveJob::Arguments.float(value)
    else
      ActiveJob::Arguments.str(value.to_s)
    end
  end

  # One native job of a bulk push, held until its batch is full.
  class HeldNative
    def initialize(job_class, retry_json, queue, extra, args_json)
      @job_class = job_class
      @retry_json = retry_json
      @queue = queue
      @extra = extra
      @args_json = args_json
    end

    def job_class
      @job_class
    end

    def retry_json
      @retry_json
    end

    def queue
      @queue
    end

    def extra
      @extra
    end

    def args_json
      @args_json
    end
  end

  # A bulk push (`perform_bulk`, the sidekiq-bulk gem's `push_bulk`):
  # its jobs are held and pushed `limit` at a time, as Sidekiq's
  # `Client#push_bulk` pushes a batch: one `SADD queues` and one `LPUSH`
  # of every envelope, which share one `created_at` and one
  # `enqueued_at`, with `class` before `args`.
  #
  # Each bulk push has its own batch on this thread's stack, so one
  # nested in another's block neither pushes nor closes the outer one.
  # `bulk_begin` opens it; `bulk_end`, once the block has built every
  # job, pushes what is left; `bulk_abort`, on every exit, closes it,
  # dropping a slice the block raised in.
  class BulkBatch
    def initialize(limit)
      @limit = limit
      @jobs = [HeldNative.new("", "", "", "", "")]
      @jobs.clear
    end

    def limit
      @limit
    end

    def jobs
      @jobs
    end
  end

  def self.bulk_begin(limit)
    bulks.push(BulkBatch.new(limit))
    nil
  end

  def self.bulk_end
    open = bulks
    if open.length > 0
      flush_batch(open[open.length - 1].jobs)
    end
    nil
  end

  def self.bulk_abort
    open = bulks
    if open.length > 0
      open.pop
    end
    nil
  end

  # This thread's open bulk pushes, innermost last.
  def self.bulks
    b = Thread.current[:sidekiq_bulks]
    if b.nil?
      b = [BulkBatch.new(0)]
      b.clear
      Thread.current[:sidekiq_bulks] = b
    end
    b
  end

  # Push a batch's held jobs, emptying it.
  def self.flush_batch(held)
    if held.length == 0
      return nil
    end
    jobs = held.dup
    held.clear
    created = now_ms.to_s
    enqueued = now_ms.to_s
    queue = jobs[0].queue + ""
    envelopes = jobs.map do |j|
      "{\"retry\":" + j.retry_json +
        ",\"queue\":" + ActiveJob::Arguments.str(j.queue) +
        j.extra +
        ",\"class\":" + ActiveJob::Arguments.str(j.job_class) +
        ",\"args\":" + j.args_json +
        ",\"jid\":" + ActiveJob::Arguments.str(SecureRandom.hex(12)) +
        ",\"created_at\":" + created +
        ",\"enqueued_at\":" + enqueued + "}"
    end
    push_many(queue, envelopes)
  end

  def self.push_many(queue, jsons)
    if RECORDING[0]
      RECORDED.push("sadd queues " + queue)
      RECORDED.push("lpush queue:" + queue + " " + jsons.join(" "))
      return nil
    end
    SidekiqRedis.push_many(url, queue, jsons)
  end

  # `enqueue`, held while a transaction is open and pushed at its COMMIT
  # (a job that sets `enqueue_after_transaction_commit = true`).
  def self.enqueue_after_commit(job_class, queue, at, aj_json)
    if ActiveJob.in_transaction
      held.push(HeldPush.new(job_class, queue, at, aj_json))
    else
      enqueue(job_class, queue, at, aj_json)
    end
    nil
  end

  def self.held
    h = Thread.current[:sidekiq_held_until_commit]
    if h.nil?
      h = [HeldPush.new("", "", 0.0, "")]
      h.clear
      Thread.current[:sidekiq_held_until_commit] = h
    end
    h
  end

  # At COMMIT: the hold is emptied first, so a push that fails can't
  # leak held jobs into this thread's next transaction. Every held job
  # is tried, and the first error is raised after the rest.
  def self.flush_held
    h = held
    if h.length == 0
      return nil
    end
    jobs = h.dup
    h.clear
    failed = false
    first = StandardError.new("")
    jobs.each do |j|
      begin
        # `queue` is an attr-reader String; push only reads it (SADD /
        # LPUSH key material). Concatenating "" is a new string so
        # Spinel's share analysis does not hand the shared handle into
        # those calls.
        enqueue(j.job_class, j.queue + "", j.at, j.aj_json)
      rescue StandardError => e
        if !failed
          failed = true
          first = e
        end
      end
    end
    if failed
      raise first
    end
    nil
  end

  def self.drop_held
    held.clear
    nil
  end

  def self.push(queue, json)
    if RECORDING[0]
      RECORDED.push("sadd queues " + queue)
      RECORDED.push("lpush queue:" + queue + " " + json)
      return nil
    end
    SidekiqRedis.push(url, queue, json)
  end

  def self.schedule(at, json)
    if RECORDING[0]
      RECORDED.push("zadd schedule " + at.to_s + " " + json)
      return nil
    end
    SidekiqRedis.schedule(url, at, json)
  end

  # Sidekiq's clock for created_at and enqueued_at: integer epoch
  # milliseconds.
  def self.now_ms
    Process.clock_gettime(Process::CLOCK_REALTIME, :millisecond)
  end

  # The app's `configure_client` URL.
  def self.url
    if URL_PID[0] != Process.pid
      URL[0] = resolve_url
      URL_PID[0] = Process.pid
    end
    URL[0]
  end

  def self.resolve_url
    configured = JobRegistry.sidekiq_redis_url
    if configured == ""
      raise ArgumentError.new("Sidekiq.configure_client's Redis url: is empty (is the environment variable it reads set?); there is no fallback to REDIS_PROVIDER, REDIS_URL or localhost")
    end
    configured
  end

  # The app's `configure_client` `password:`, `""` for none (the URL's
  # own password, if any, then stands).
  def self.password
    JobRegistry.sidekiq_redis_password
  end

  # The app's `configure_client` `username:`, `""` for none (the URL's
  # own user, if any, then stands).
  def self.username
    JobRegistry.sidekiq_redis_username
  end

  # The app's `configure_client` `db:`, -1 for none (the URL's database
  # then stands).
  def self.db
    JobRegistry.sidekiq_redis_db
  end

  # The connect and read timeouts, in seconds: the app's `timeout:`,
  # `connect_timeout:` and `read_timeout:`, or Sidekiq's 3.
  def self.connect_timeout
    JobRegistry.sidekiq_redis_connect_timeout
  end

  def self.read_timeout
    JobRegistry.sidekiq_redis_read_timeout
  end

  # The seconds to wait before each retry after a connection error, from
  # the app's `reconnect_attempts:`, or Sidekiq's one immediate retry.
  def self.reconnect_delays
    delays = []
    JobRegistry.sidekiq_redis_reconnect_delays.split(" ").each do |word|
      delays.push(word.to_f)
    end
    delays
  end

  # A `db:` read from the environment: its Integer, or -1 when the
  # variable is unset or empty, so the URL's database stands as it does
  # for redis-client's nil `db:`. Anything else raises, as redis-client's
  # `Integer(db)` does.
  def self.db_setting(value)
    if value == ""
      return -1
    end
    i = 0
    while i < value.length
      if !"0123456789".include?(value[i])
        raise ArgumentError.new("db: must be an Integer, got: \"" + value + "\" (Sidekiq.configure_client's Redis setting)")
      end
      i += 1
    end
    value.to_i
  end

  # A `configure_client` setting read from the environment, for the
  # registry's generated `sidekiq_redis_url` and `_password`:
  # `ENV["NAME"]`, `""` when unset.
  def self.setting(name)
    ENV[name].to_s
  end

  # `ENV.fetch("NAME", "default")`. An empty variable counts as unset.
  def self.setting_or(name, default)
    value = ENV[name].to_s
    value == "" ? default : value
  end

  # `ENV.fetch("NAME")`, which raises when the variable is unset, as the
  # app's initializer would at boot.
  def self.required_setting(name)
    value = ENV[name].to_s
    if value == ""
      raise KeyError.new("key not found: \"" + name + "\" (Sidekiq.configure_client's Redis setting)")
    end
    value
  end

  # The URL with its password replaced by REDACTED, for the boot line,
  # as Sidekiq's own `scrub` does.
  def self.redacted_url(value)
    scheme = value.index("://")
    if scheme.nil?
      return value
    end
    start = scheme + 3
    tail = value[start, value.length - start]
    at = tail.rindex("@")
    if at.nil?
      return value
    end
    userinfo = tail[0, at]
    colon = userinfo.index(":")
    shown = colon.nil? ? "REDACTED" : userinfo[0, colon] + ":REDACTED"
    value[0, start] + shown + tail[at, tail.length - at]
  end

  # The boot line a server on `:sidekiq` logs. It doesn't connect:
  # Sidekiq's client connects at the first push too.
  def self.announce
    warn "[job] queue_adapter :sidekiq: jobs are pushed to " + redacted_url(url) + "; run them with this build's sidekiq role or a Sidekiq process"
    nil
  end

  def self.start_recording
    RECORDING[0] = true
    RECORDED.clear
    nil
  end

  def self.stop_recording
    RECORDING[0] = false
    nil
  end

  def self.recorded
    RECORDED.dup
  end
end
