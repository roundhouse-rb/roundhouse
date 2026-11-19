//! Native Sidekiq workers under `queue_adapter :sidekiq`
//! (`lower::sidekiq_native`): a class that includes `Sidekiq::Job` or
//! `Sidekiq::Worker` pushes its jobs to Redis in Sidekiq's own envelope.
//!
//! The pushes are held to `tests/sidekiq_native/sidekiq.json`, which
//! `tests/sidekiq_native/oracle.rb` records from real Sidekiq 8.1.7's
//! client against a real Redis. Each call form there is written here as
//! app code, so it goes through the fold and the generated writer, and
//! the emitted tree's pushes are recorded through `SidekiqPush`'s seam (no
//! Redis). Every envelope must match the oracle's byte for byte once its
//! `jid`, `created_at` and `enqueued_at` are pinned to the oracle's
//! values, and every ZADD's score must be `Float#to_s` of the time the
//! oracle scheduled at.

use roundhouse::project::BuildTarget;
use serde_json::Value;

use super::emit_and_run;

/// The oracle's workers (`tests/sidekiq_native/oracle_jobs.rb`), with a
/// positional `perform` that takes what their scenarios pass: the oracle
/// writes `perform(*)`, which Sidekiq never calls on the client side.
const WORKERS: &[(&str, &str)] = &[
    (
        "app/workers/plain_worker.rb",
        "class PlainWorker\n  include Sidekiq::Job\n\n  def perform(a = nil, b = nil, c = {}, d = [], e = nil, f = nil, g = nil, h = nil, i = nil)\n    nil\n  end\nend\n",
    ),
    (
        "app/workers/ok_worker.rb",
        "class OkWorker\n  include Sidekiq::Job\n  sidekiq_options queue: \"pull\", retry: 5\n\n  def perform(id, opts = {})\n    nil\n  end\nend\n",
    ),
    (
        "app/workers/options_worker.rb",
        "class OptionsWorker\n  include Sidekiq::Worker\n  sidekiq_options dead: false, backtrace: true, queue: \"push\", retry: 8,\n    retry_queue: \"low\", retry_for: 3600, lock: :until_executed, lock_ttl: 86_400\n\n  def perform(n)\n    nil\n  end\nend\n",
    ),
    (
        "app/workers/child_worker.rb",
        "class ChildWorker < OptionsWorker\n  sidekiq_options queue: \"child\", backtrace: 5, tags: [\"x\"]\nend\n",
    ),
    (
        "app/workers/no_retry_worker.rb",
        "class NoRetryWorker\n  include Sidekiq::Job\n  sidekiq_options retry: false, queue: :default\n\n  def perform(n)\n    nil\n  end\nend\n",
    ),
];

/// The bulk scenarios' `OkWorker`: its `push_bulk` passes a String second
/// argument where the other scenarios pass a Hash, and a parameter has
/// one type here.
const BULK_OK_WORKER: &str =
    "class OkWorker\n  include Sidekiq::Job\n  sidekiq_options queue: \"pull\", retry: 5\n\n  def perform(id, label = nil)\n    nil\n  end\nend\n";

/// The bulk pushes: `perform_bulk` and the sidekiq-bulk gem's `push_bulk`.
const BULK_SCENARIOS: &[(&str, &str)] = &[
    ("perform_bulk", "OkWorker.perform_bulk([[1], [2], [3]])"),
    ("perform_bulk_batches", "OkWorker.perform_bulk([[1], [2], [3], [4], [5]], batch_size: 2)"),
    ("perform_bulk_set", "OkWorker.set(queue: \"other\").perform_bulk([[1], [2]])"),
    ("push_bulk_block", "OkWorker.push_bulk([1, 2, 3, 4, 5], limit: 2) { |i| [i, \"n#{i}\"] }"),
    ("push_bulk_no_block", "PlainWorker.push_bulk([1, 2, 3])"),
    ("push_bulk_all_options", "OptionsWorker.push_bulk([1, 2])"),
];

/// `push_bulk` over a relation's records, as Mastodon pushes them: a
/// `find_in_batches` batch, a `where(...)` chain filtered with `reject`,
/// and an association filtered with `select`. Each block reads the
/// record's `id`, which `push_bulk` (a gem method the analyzer doesn't
/// know) would otherwise leave untyped. The setup makes the pushed ids
/// 1, 2 and 3, so each push is the oracle's `push_bulk_no_block`
/// (`PlainWorker.push_bulk([1, 2, 3])`) byte for byte.
const RELATION_SCENARIOS: &[(&str, &str)] = &[
    (
        "relation_find_in_batches",
        // Ordered, as Mastodon's batches are (`reorder(nil)`): a bare
        // `where(...).find_in_batches` is folded to SQL and answers an
        // Array, which has no `find_in_batches` in the runtime (a gap
        // apart from this push).
        r##"Article.where(body: "A long enough article body.").order(:id).find_in_batches do |batch|
      PlainWorker.push_bulk(batch) { |article| [article.id] }
    end"##,
    ),
    (
        "relation_where_reject",
        // Through `to_a`: the runtime's Relation has no `reject` (a gap
        // apart from this push).
        r##"kept = Article.where(id: [1, 2, 3, 4]).to_a
    kept = kept.reject { |article| article.title == "Skipped" }
    PlainWorker.push_bulk(kept, limit: 1_000) do |article|
      [article.id]
    end"##,
    ),
    (
        "association_select",
        r##"article = Article.find(1)
    PlainWorker.push_bulk(article.comments.select { |comment| comment.body.length > 0 }) { |comment| [comment.id] }"##,
    ),
];

const RELATION_SETUP: &str = r##"SidekiqPush.start_recording
3.times { |i| Article.create!(title: "Article #{i}", body: "A long enough article body.") }
Article.create!(title: "Skipped", body: "Another long enough body.")
3.times { |i| Comment.create!(article_id: 1, commenter: "c#{i}", body: "Comment body #{i}") }
"##;

/// The oracle scenario a scenario here is held to, when not its own name.
fn oracle_name(name: &str) -> &str {
    if RELATION_SCENARIOS.iter().any(|(n, _)| *n == name) { "push_bulk_no_block" } else { name }
}

/// Each oracle scenario this covers, and its call as app code. The
/// arguments strict mode rejects are compile errors instead (below).
const SCENARIOS: &[(&str, &str)] = &[
    ("async_plain", "PlainWorker.perform_async(1)"),
    ("async_options", "OkWorker.perform_async(1, { \"a\" => \"b\" })"),
    (
        "async_all_types",
        "PlainWorker.perform_async(1, \"x\", { \"a\" => \"b\" }, [2, 3], nil, 1.5, true, false, \"\u{e9}\\\"\\\\\\n\")",
    ),
    ("async_no_args", "PlainWorker.perform_async"),
    ("async_all_options", "OptionsWorker.perform_async(7)"),
    ("async_inherited", "ChildWorker.perform_async(8)"),
    ("async_retry_false", "NoRetryWorker.perform_async(9)"),
    ("in_interval", "OkWorker.perform_in(300, 2)"),
    ("in_float", "PlainWorker.perform_in(1.5, 2)"),
    ("in_epoch", "OkWorker.perform_in(2000000000, 2)"),
    ("at_epoch", "OkWorker.perform_at(2000000000, 3)"),
    ("at_time", "OkWorker.perform_at(Time.at(2000000000), 3)"),
    ("in_zero", "OkWorker.perform_in(0, 4)"),
    ("at_past", "OkWorker.perform_at(Time.now - 60, 4)"),
    ("in_all_options", "OptionsWorker.perform_in(60, 5)"),
    ("set_queue_async", "OkWorker.set(queue: \"other\").perform_async(1)"),
    ("set_queue_sym_async", "PlainWorker.set(queue: :other).perform_async(1)"),
    ("set_queue_in", "OkWorker.set(queue: \"other\").perform_in(60, 1)"),
    ("set_queue_at", "OkWorker.set(queue: \"other\").perform_at(2000000000, 1)"),
    ("set_wait_async", "OkWorker.set(wait: 60).perform_async(1)"),
    ("set_queue_wait_async", "OkWorker.set(queue: \"other\", wait: 60).perform_async(1)"),
    ("set_wait_until_async", "OkWorker.set(wait_until: 2000000000).perform_async(1)"),
    ("set_retry_async", "OkWorker.set(retry: false).perform_async(1)"),
    ("set_queue_all_options", "OptionsWorker.set(queue: \"other\").perform_async(1)"),
    ("set_chained", "OkWorker.set(queue: \"a\").set(queue: \"b\", wait: 60).perform_async(1)"),
];

const SIDEKIQ: &str = "  config.active_job.queue_adapter = :sidekiq\n";
const REDIS_CLIENT: &str = "Sidekiq.configure_client do |config|\n  config.redis = { url: ENV.fetch(\"REDIS_URL\", \"redis://localhost:6379/0\") }\nend\n";

fn with_workers(mut overlay: emit_and_run::Overlay) -> emit_and_run::Overlay {
    for (path, source) in WORKERS {
        overlay = overlay.write(path, source);
    }
    overlay
}

/// The blog on `:sidekiq` with the oracle's workers, and one class method
/// on `Article` per scenario that makes its call.
fn on_sidekiq(scenarios: &[(&str, &str)]) -> emit_and_run::Overlay {
    let methods: String = scenarios
        .iter()
        .map(|(name, call)| format!("\n  def self.native_{name}\n    {call}\n  end\n"))
        .collect();
    with_workers(emit_and_run::real_blog())
        .edit(
            "app/models/article.rb",
            "  validates :title, presence: true\n",
            &format!("  validates :title, presence: true\n{methods}"),
        )
        .edit("config/environments/production.rb", "  config.active_job.queue_adapter = :solid_queue\n", SIDEKIQ)
        .write("config/initializers/sidekiq.rb", REDIS_CLIENT)
}

fn script(scenarios: &[(&str, &str)]) -> String {
    script_after(scenarios, "")
}

/// [`script`], after `setup` (records the scenarios read).
fn script_after(scenarios: &[(&str, &str)], setup: &str) -> String {
    let names: Vec<String> = scenarios.iter().map(|(n, _)| format!("\"{n}\"")).collect();
    format!(
        "ActiveJob.register_drain\n{setup}\
         [{}].each do |name|\n\
         \x20 SidekiqPush.start_recording\n\
         \x20 t = Time.now.to_f\n\
         \x20 Article.send(\"native_\" + name)\n\
         \x20 puts \"push \" + name + \" \" + t.to_s\n\
         \x20 SidekiqPush.recorded.each {{ |line| puts \"cmd \" + line }}\n\
         end\n\
         SidekiqPush.stop_recording\n\
         raise \"queued in process too\" unless ActiveJob.pending_count == 0\n\
         puts \"native pushes recorded\"\n",
        names.join(", ")
    )
}

/// The pinned values the oracle writes into each envelope's `pinned` text.
const PINNED_JID: &str = "0123456789abcdef01234567";
const PINNED_CREATED: &str = "1792632730123";
const PINNED_ENQUEUED: &str = "1792632730124";

/// The envelope with `jid`, `created_at` and `enqueued_at` pinned, after
/// checking each has Sidekiq's shape. Also answers `created_at` in ms.
fn pin(raw: &str) -> (String, i64) {
    let field = |key: &str| -> (usize, usize) {
        let at = raw.find(&format!("\"{key}\":")).unwrap_or_else(|| panic!("no {key}: {raw}"));
        let start = at + key.len() + 3;
        let len = raw[start..].find([',', '}']).unwrap();
        (start, start + len)
    };
    let mut out = raw.to_string();
    let (s, e) = field("jid");
    let jid = &raw[s..e];
    assert!(
        jid.len() == 26 && jid[1..25].bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "jid: {jid}"
    );
    let (cs, ce) = field("created_at");
    let created: i64 = raw[cs..ce].parse().unwrap_or_else(|_| panic!("created_at isn't integer ms: {raw}"));
    assert_eq!(raw[cs..ce].len(), 13, "created_at isn't epoch ms: {raw}");
    // Right to left, so the earlier offsets still hold.
    if raw.contains("\"enqueued_at\":") {
        let (es, ee) = field("enqueued_at");
        assert!(raw[es..ee].parse::<i64>().is_ok(), "enqueued_at isn't integer ms: {raw}");
        out.replace_range(es..ee, PINNED_ENQUEUED);
    }
    out.replace_range(cs..ce, PINNED_CREATED);
    out.replace_range(s..e, &format!("\"{PINNED_JID}\""));
    (out, created)
}

/// The JSON texts of a recorded `lpush`, which holds a batch's envelopes
/// separated by spaces.
fn envelopes(raw: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut stream = serde_json::Deserializer::from_str(raw).into_iter::<Value>();
    let mut start = 0;
    while let Some(v) = stream.next() {
        v.unwrap_or_else(|e| panic!("not JSON ({e}): {raw}"));
        let end = stream.byte_offset();
        out.push(raw[start..end].trim());
        start = end;
    }
    out
}

/// Run `scenarios` on the emitted tree and hold each one's pushes to the
/// oracle's, as the module doc says.
fn check_against_oracle(run: &emit_and_run::Run, scenarios: &[(&str, &str)]) {
    let oracle: Value = serde_json::from_str(
        &std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sidekiq_native/sidekiq.json"))
            .unwrap(),
    )
    .expect("sidekiq.json is JSON");
    assert_eq!(oracle["versions"]["sidekiq"], "8.1.7", "the oracle must come from Sidekiq 8.1.7");

    // name → (call time, recorded commands)
    let mut got: Vec<(String, f64, Vec<String>)> = Vec::new();
    for line in run.stdout.lines() {
        if let Some(rest) = line.strip_prefix("push ") {
            let (name, t) = rest.split_once(' ').unwrap();
            got.push((name.to_string(), t.parse().unwrap(), Vec::new()));
        } else if let Some(cmd) = line.strip_prefix("cmd ") {
            got.last_mut().expect("a cmd before any push").2.push(cmd.to_string());
        }
    }
    assert_eq!(got.len(), scenarios.len(), "{}", run.stdout);
    for (name, called_at, cmds) in &got {
        let want = oracle["scenarios"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == oracle_name(name))
            .unwrap_or_else(|| panic!("{name}: not in the oracle"));
        let wcmds: Vec<&Value> = want["round_trips"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| t["commands"].as_array().unwrap())
            .collect();
        assert_eq!(cmds.len(), wcmds.len(), "{name}: command count\n{cmds:#?}");
        for (c, wc) in cmds.iter().zip(wcmds) {
            let argv = wc["argv"].as_array().unwrap();
            let verb = argv[0].as_str().unwrap();
            let key = argv[1].as_str().unwrap();
            match verb {
                "sadd" => assert_eq!(c, &format!("sadd {key} {}", argv[2].as_str().unwrap()), "{name}"),
                "lpush" => {
                    let raw = c.strip_prefix(&format!("lpush {key} ")).unwrap_or_else(|| panic!("{name}: {c}"));
                    let got_envs = envelopes(raw);
                    let want_envs = wc["envelopes"].as_array().unwrap();
                    assert_eq!(got_envs.len(), want_envs.len(), "{name}: envelopes in the batch\n{raw}");
                    let mut created = std::collections::BTreeSet::new();
                    let mut jids = std::collections::BTreeSet::new();
                    for (g, w) in got_envs.iter().zip(want_envs) {
                        let (pinned, at) = pin(g);
                        created.insert(at);
                        jids.insert(g[g.find("\"jid\":").unwrap()..][..32].to_string());
                        assert_eq!(pinned, w["pinned"].as_str().unwrap(), "{name}: envelope");
                    }
                    if got_envs.len() > 1 {
                        assert_eq!(created.len(), 1, "{name}: a batch shares one created_at");
                        assert_eq!(jids.len(), got_envs.len(), "{name}: each job has its own jid");
                    }
                }
                "zadd" => {
                    let rest = c.strip_prefix(&format!("zadd {key} ")).unwrap_or_else(|| panic!("{name}: {c}"));
                    let (score, raw) = rest.split_once(' ').unwrap();
                    let at: f64 = score.parse().unwrap_or_else(|_| panic!("{name}: score {score}"));
                    assert!(score.contains('.'), "{name}: score {score} isn't Float#to_s");
                    let offset = wc["at_minus_call_time_s"].as_f64().unwrap();
                    // An epoch (the oracle's 2,000,000,000) is absolute; its
                    // recorded offset is from the oracle's own call time.
                    if offset > 1e8 {
                        assert_eq!(score, "2000000000.0", "{name}: an epoch is scheduled at itself");
                    } else {
                        assert!((at - called_at - offset).abs() < 2.0, "{name}: scheduled {} s out, want {offset}", at - called_at);
                    }
                    let (pinned, _) = pin(raw);
                    assert_eq!(pinned, wc["envelopes"][0]["pinned"].as_str().unwrap(), "{name}: envelope");
                }
                other => panic!("{name}: unexpected command {other}"),
            }
        }
    }
}

#[test]
fn native_pushes_match_sidekiq() {
    let run = on_sidekiq(SCENARIOS).run_ruby(&script(SCENARIOS));
    run.assert_passes();
    assert!(run.stdout.contains("native pushes recorded"), "{}", run.stdout);
    check_against_oracle(&run, SCENARIOS);

    let registry = std::fs::read_to_string(run.emitted.join("runtime/job_registry.rb")).unwrap();
    assert!(
        registry.contains("def self.push_native_ok_worker(queue, at, retry_json, shape, argc, id, opts)"),
        "{registry}"
    );
    let worker = std::fs::read_to_string(run.emitted.join("app/workers/ok_worker.rb"))
        .or_else(|_| std::fs::read_to_string(run.emitted.join("app/models/ok_worker.rb")))
        .unwrap();
    assert!(worker.contains("def self.perform_async_with(at, queue, retry_json, shape, argc, id, opts = {})"), "{worker}");
    assert!(!worker.contains("Sidekiq::Job") && !worker.contains("sidekiq_options"), "{worker}");
}

/// `perform_bulk` and `push_bulk` push in batches, each one `SADD` and one
/// `LPUSH` of the batch's envelopes, as Sidekiq's `Client#push_bulk`.
#[test]
fn bulk_pushes_match_sidekiq() {
    let run = on_sidekiq(BULK_SCENARIOS)
        .write("app/workers/ok_worker.rb", BULK_OK_WORKER)
        .run_ruby(&script(BULK_SCENARIOS));
    run.assert_passes();
    assert!(run.stdout.contains("native pushes recorded"), "{}", run.stdout);
    check_against_oracle(&run, BULK_SCENARIOS);
}

/// A `push_bulk` block's other pushes go out as they would anywhere (a
/// `perform_in` is scheduled, another worker's push isn't batched), and
/// a block that raises pushes the slices it finished and drops the one
/// it raised in, as sidekiq-bulk does.
#[test]
fn a_bulk_block_holds_only_its_own_jobs() {
    let run = on_sidekiq(&[
        (
            "bulk_with_others",
            "OkWorker.push_bulk([1, 2, 3], limit: 2) do |i|\n      PlainWorker.perform_in(60, i)\n      NoRetryWorker.perform_async(i)\n      [i, \"n\"]\n    end",
        ),
        (
            "bulk_raises",
            "OkWorker.push_bulk([1, 2, 3], limit: 2) do |i|\n      raise ArgumentError, \"bad item\" if i == 3\n      [i, \"n\"]\n    end",
        ),
    ])
    .write("app/workers/ok_worker.rb", BULK_OK_WORKER)
    .run_ruby(
        r##"ActiveJob.register_drain
SidekiqPush.start_recording
Article.native_bulk_with_others
lines = SidekiqPush.recorded
scheduled = lines.select { |l| l.start_with?("zadd schedule ") }
raise "perform_in in the block: #{lines.inspect}" unless scheduled.length == 3 && scheduled.all? { |l| l.include?('"class":"PlainWorker"') }
others = lines.select { |l| l.start_with?("lpush queue:default ") }
raise "another worker's push: #{lines.inspect}" unless others.length == 3 && others.all? { |l| l.split('"jid":').length == 2 }
bulk = lines.select { |l| l.start_with?("lpush queue:pull ") }
raise "the bulk's batches: #{lines.inspect}" unless bulk.map { |l| l.split('"jid":').length - 1 } == [2, 1]
SidekiqPush.start_recording
raised = false
begin
  Article.native_bulk_raises
rescue ArgumentError
  raised = true
end
lines = SidekiqPush.recorded
raise "the block's error: #{lines.inspect}" unless raised
raise "a raised slice was pushed: #{lines.inspect}" unless lines.length == 2 && lines[1].split('"jid":').length == 3 && !lines[1].include?('"args":[3,')
raise "the batch stayed open" unless SidekiqPush.bulks.length == 0
SidekiqPush.stop_recording
puts "bulk block passed"
"##,
    );
    run.assert_passes();
    assert!(run.stdout.contains("bulk block passed"), "{}", run.stdout);
}

/// A bulk push over a relation's records types each item as the model,
/// so `item.id` is an Integer and the push is Sidekiq's.
#[test]
fn bulk_pushes_over_relations_match_sidekiq() {
    let run = on_sidekiq(RELATION_SCENARIOS).run_ruby(&script_after(RELATION_SCENARIOS, RELATION_SETUP));
    run.assert_passes();
    assert!(run.stdout.contains("native pushes recorded"), "{}", run.stdout);
    check_against_oracle(&run, RELATION_SCENARIOS);
}

const MIXED_WORKER: &str = "class MixedWorker\n  include Sidekiq::Job\n\n  def perform(label, opts = {})\n    nil\n  end\nend\n";

const MIXED_SCENARIOS: &[(&str, &str)] = &[
    ("mixed_hash", r##"MixedWorker.perform_async("x", { "s" => "a\"b", "i" => 2, "f" => 1.5, "t" => true, "u" => false, "n" => nil })"##),
    ("mixed_label", "MixedWorker.perform_async(true)"),
];

/// A String-keyed Hash whose values are different JSON scalars, and a
/// parameter that is a String at one call site and a Boolean at another,
/// are written as Sidekiq writes them: its envelope's `args` is
/// `JSON.generate` of the arguments. No oracle scenario has these, so
/// this holds them to `JSON.generate` itself.
#[test]
fn mixed_scalar_arguments_write_what_json_generate_writes() {
    let run = on_sidekiq(MIXED_SCENARIOS).write("app/workers/mixed_worker.rb", MIXED_WORKER).run_ruby(
        r##"ActiveJob.register_drain
SidekiqPush.start_recording
Article.native_mixed_hash
Article.native_mixed_label
envs = SidekiqPush.recorded.select { |l| l.start_with?("lpush ") }
raise "pushes: #{SidekiqPush.recorded.inspect}" unless envs.length == 2
want = [
  JSON.generate(["x", { "s" => "a\"b", "i" => 2, "f" => 1.5, "t" => true, "u" => false, "n" => nil }]),
  JSON.generate([true]),
]
want.each_with_index do |w, i|
  raise "#{envs[i]} doesn't write #{w}" unless envs[i].include?("\"args\":" + w + ",\"class\":\"MixedWorker\"")
end
puts "mixed passed"
"##,
    );
    run.assert_passes();
    assert!(run.stdout.contains("mixed passed"), "{}", run.stdout);
    let registry = std::fs::read_to_string(run.emitted.join("runtime/job_registry.rb")).unwrap();
    assert!(registry.contains("SidekiqPush.native_value(label)"), "{registry}");
}

/// Under `:sidekiq` a worker whose queue or retry can't be read, its own
/// or inherited, is an error: the push would write the default in its
/// place. So is enqueueing a worker that has its own `perform_async`,
/// defined on it or from a module it extends, which isn't lowered to a
/// push.
#[test]
fn unreadable_options_and_an_own_perform_async_are_errors_under_sidekiq() {
    let errors_for = |files: &[(&str, &str)], call: &str| {
        let mut overlay = with_workers(emit_and_run::real_blog());
        for (path, source) in files {
            overlay = overlay.write(path, source);
        }
        let (_, errors) = overlay
            .edit(
                "app/models/article.rb",
                "  validates :title, presence: true\n",
                &format!("  validates :title, presence: true\n\n  def self.enqueue_native\n    {call}\n  end\n"),
            )
            .edit("config/environments/production.rb", "  config.active_job.queue_adapter = :solid_queue\n", SIDEKIQ)
            .write("config/initializers/sidekiq.rb", REDIS_CLIENT)
            .emit(BuildTarget::Ruby);
        errors
    };
    let perform = "\n  def perform(n)\n    nil\n  end\nend\n";
    let queue_worker = format!("class QueueWorker\n  include Sidekiq::Job\n  sidekiq_options queue: ENV.fetch(\"Q\", \"low\")\n{perform}");
    let base_worker = "class BaseWorker\n  include Sidekiq::Job\n  sidekiq_options retry: ENV.fetch(\"R\", \"3\").to_i\nend\n";
    let sub_worker = format!("class SubWorker < BaseWorker\n{perform}");
    let hash_worker = format!("class HashWorker\n  include Sidekiq::Job\n  sidekiq_options(Hash.new)\n{perform}");
    let custom_worker =
        format!("class CustomWorker\n  include Sidekiq::Job\n\n  def self.perform_async(n)\n    new.perform(n)\n  end\n{perform}");
    let fast_enqueue = "module FastEnqueue\n  def perform_async(n)\n    nil\n  end\nend\n";
    let extended_worker = format!("class ExtendedWorker\n  include Sidekiq::Job\n  extend FastEnqueue\n{perform}");
    // `extend Enqueue` in `Feeds::NestedWorker` names `Feeds::Enqueue`.
    let feeds_enqueue = "module Feeds\n  module Enqueue\n    def perform_async(n)\n      nil\n    end\n  end\nend\n";
    let nested_worker =
        format!("module Feeds\n  class NestedWorker\n    include Sidekiq::Job\n    extend Enqueue\n\n    def perform(n)\n      nil\n    end\n  end\nend\n");
    let cases: Vec<(Vec<(&str, &str)>, &str, &str)> = vec![
        (vec![("app/workers/queue_worker.rb", queue_worker.as_str())], "QueueWorker.perform_async(1)", "sidekiq_options `queue:` is not a literal"),
        (
            vec![("app/workers/base_worker.rb", base_worker), ("app/workers/sub_worker.rb", sub_worker.as_str())],
            "SubWorker.perform_async(1)",
            "sidekiq_options `retry:` (inherited from `BaseWorker`) is not a literal",
        ),
        (vec![("app/workers/hash_worker.rb", hash_worker.as_str())], "HashWorker.perform_async(1)", "is not one literal Hash"),
        (vec![("app/workers/custom_worker.rb", custom_worker.as_str())], "CustomWorker.perform_in(60, 1)", "`CustomWorker` defines its own `perform_async`"),
        (vec![("app/workers/custom_worker.rb", custom_worker.as_str())], "CustomWorker.perform_async(1)", "`CustomWorker` defines its own `perform_async`"),
        (
            vec![("app/workers/fast_enqueue.rb", fast_enqueue), ("app/workers/extended_worker.rb", extended_worker.as_str())],
            "ExtendedWorker.perform_async(1)",
            "`ExtendedWorker` gets its own `perform_async` from `FastEnqueue`",
        ),
        (
            vec![("app/workers/feeds/enqueue.rb", feeds_enqueue), ("app/workers/feeds/nested_worker.rb", nested_worker.as_str())],
            "Feeds::NestedWorker.perform_async(1)",
            "`Feeds::NestedWorker` gets its own `perform_async` from `Feeds::Enqueue`",
        ),
    ];
    for (files, call, want) in cases {
        let errors = errors_for(&files, call);
        assert!(errors.iter().any(|e| e.contains(want)), "{call}: {errors:#?}");
    }
}

/// An app not on `:sidekiq` keeps #746's inline workers: no
/// `perform_async_with`, no native writer, and the same tree with or
/// without this pass's planning.
#[test]
fn off_sidekiq_workers_stay_inline() {
    let (emitted, errors) = with_workers(emit_and_run::real_blog())
        .edit(
            "app/models/article.rb",
            "  validates :title, presence: true\n",
            "  validates :title, presence: true\n\n  def self.enqueue_native\n    OkWorker.perform_in(300, 2)\n  end\n",
        )
        .emit(BuildTarget::Ruby);
    assert!(errors.is_empty(), "{errors:#?}");
    let registry = std::fs::read_to_string(emitted.join("runtime/job_registry.rb")).unwrap();
    assert!(!registry.contains("push_native_"), "{registry}");
    let model = std::fs::read_to_string(emitted.join("app/models/article.rb")).unwrap();
    assert!(!model.contains("perform_async_with") && model.contains("OkWorker.perform_async(2)"), "{model}");
}

/// Arguments Sidekiq's strict mode refuses at push time are compile
/// errors under `:sidekiq`, naming the parameter and its type, as are
/// `set(...)` options that can't be honored.
#[test]
fn non_json_arguments_are_errors_under_sidekiq() {
    let rejected = |call: &str| {
        let (_, errors) = with_workers(emit_and_run::real_blog())
            .edit(
                "app/models/article.rb",
                "  validates :title, presence: true\n",
                &format!("  validates :title, presence: true\n\n  def self.enqueue_native\n    {call}\n  end\n"),
            )
            .edit("config/environments/production.rb", "  config.active_job.queue_adapter = :solid_queue\n", SIDEKIQ)
            .write("config/initializers/sidekiq.rb", REDIS_CLIENT)
            .emit(BuildTarget::Ruby);
        errors
    };
    for (call, want) in [
        ("NoRetryWorker.perform_async(:sym)", "`n` is a Symbol"),
        ("NoRetryWorker.perform_async({ a: 1 })", "`n` is a Hash with Symbol keys"),
        ("NoRetryWorker.perform_async(Time.at(0))", "`n` is a Time"),
        // A non-JSON value among a Hash's mixed scalars.
        ("NoRetryWorker.perform_async({ \"a\" => 1, \"b\" => :x })", "`n` is a Symbol"),
        ("NoRetryWorker.set(priority: 3).perform_async(1)", "must be literal `queue:`"),
    ] {
        let errors = rejected(call);
        assert!(errors.iter().any(|e| e.contains(want)), "{call}: {errors:#?}");
    }
}
