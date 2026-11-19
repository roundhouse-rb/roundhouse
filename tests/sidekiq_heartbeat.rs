//! The worker's heartbeat (`SidekiqHeartbeat` in
//! `runtime/spinel/sidekiq/sidekiq_worker.rb`) and the poller's process
//! cleanup, held to what a real Sidekiq 8.1.7 process writes to Redis
//! to be seen by the Web UI and `Sidekiq::ProcessSet`.
//!
//! `tests/sidekiq_heartbeat/oracle.rb` runs the real `sidekiq` executable
//! and records `tests/sidekiq_heartbeat/sidekiq.json`: every round trip
//! of its heartbeat thread, the poller's cleanup and the shutdown write,
//! through an idle beat, two beats with a job running, and the Web UI's
//! TTIN, TSTP and TERM. `tests/sidekiq_heartbeat/driver.rb` plays the
//! same timeline through the runtime, over the real
//! `runtime/spinel/sidekiq/sidekiq_redis.rb` and a recording client
//! (`tests/sidekiq_heartbeat/redis.rb`), with the clock and identity
//! scripted. Each round trip must match Sidekiq's command for command,
//! once the values that change per run are masked: hostname, pid,
//! nonce, times, RSS, RTT, the date and the job's own ids.
//!
//! Two differences are expected and checked as such:
//!
//! * `info`'s `labels` hold `"roundhouse"`, so the Web UI shows which
//!   processes are Roundhouse builds.
//! * The cleanup's HGET and the shutdown's SREM and UNLINK go out one
//!   command per round trip where Sidekiq pipelines them:
//!   spinel-redis's pipeline has no HGET or SREM. Redis sees the same
//!   commands.
//!
//! The spinel test compiles the same driver and requires the same
//! output as CRuby. Marked `#[ignore]`, as the other spinel tests are;
//! it needs `spinel` on PATH:
//!
//!     cargo test --test sidekiq_heartbeat -- --ignored

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const SOURCES: &[(&str, &str)] = &[
    ("runtime/spinel/base64.rb", "base64.rb"),
    ("runtime/ruby/active_job.rb", "active_job.rb"),
    ("runtime/spinel/global_id_locator.rb", "global_id_locator.rb"),
    ("runtime/spinel/active_job_serialization.rb", "active_job_serialization.rb"),
    ("runtime/spinel/sidekiq/sidekiq_redis.rb", "sidekiq_redis.rb"),
    ("runtime/spinel/sidekiq_push.rb", "sidekiq_push.rb"),
    ("runtime/spinel/job_registry.rb", "job_registry.rb"),
    ("runtime/spinel/sidekiq_envelope.rb", "sidekiq_envelope.rb"),
    ("runtime/spinel/sidekiq/sidekiq_worker.rb", "sidekiq_worker.rb"),
    ("tests/sidekiq_heartbeat/redis.rb", "redis.rb"),
    ("tests/sidekiq_heartbeat/driver.rb", "driver.rb"),
];

/// The driver's process: `SidekiqHeartbeat.setup("host-1", 4242, "0123456789ab", 1792632700.25)`.
const IDENTITY: &str = "host-1:4242:0123456789ab";
const STARTED_AT: f64 = 1_792_632_700.25;
/// The driver's first beat; each later one is 10 s on.
const FIRST_BEAT: f64 = 1_792_632_731.5;

/// Sidekiq 8.1.7's `to_data`, in its order (`launcher.rb`).
const INFO_KEYS: &[&str] = &[
    "hostname", "started_at", "pid", "tag", "capsules", "queues", "weights", "labels", "identity", "version", "embedded",
];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn oracle() -> Value {
    let oracle: Value = serde_json::from_str(
        &std::fs::read_to_string(root().join("tests/sidekiq_heartbeat/sidekiq.json")).unwrap(),
    )
    .expect("sidekiq.json is JSON");
    assert_eq!(oracle["versions"]["sidekiq"], "8.1.7", "the oracle must come from Sidekiq 8.1.7");
    oracle
}

fn scratch(name: &str) -> PathBuf {
    let base = option_env!("CARGO_TARGET_TMPDIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let dir = base.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir scratch");
    for (from, to) in SOURCES {
        std::fs::copy(root().join(from), dir.join(to)).unwrap_or_else(|e| panic!("copy {from}: {e}"));
    }
    dir
}

/// The driver under CRuby, with its directory first on the load path so
/// `require "redis"` finds the recording client, not the gem.
fn run_cruby(dir: &Path) -> String {
    let out = Command::new("ruby").args(["-I", ".", "driver.rb"]).current_dir(dir).output().expect("ruby is on PATH");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success() && stdout.lines().any(|l| l == "done"),
        "the CRuby driver did not finish\n=== stdout ===\n{stdout}\n=== stderr ===\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let failures: Vec<&str> = stdout.lines().filter(|l| l.starts_with("FAIL")).collect();
    assert!(failures.is_empty(), "runtime checks failed:\n{}", failures.join("\n"));
    stdout
}

struct Trip {
    kind: String,
    commands: Vec<Vec<String>>,
}

/// The driver's `trip` lines for one section of its timeline.
fn driver_trips(stdout: &str, section: &str) -> Vec<Trip> {
    stdout
        .lines()
        .filter_map(|l| l.strip_prefix("trip\t"))
        .filter_map(|l| {
            let mut parts = l.splitn(3, '\t');
            let (s, kind, json) = (parts.next()?, parts.next()?, parts.next()?);
            (s == section).then(|| Trip {
                kind: kind.to_string(),
                commands: serde_json::from_str(json).unwrap_or_else(|e| panic!("trip commands ({e}): {json}")),
            })
        })
        .collect()
}

/// The oracle's round trips from one thread (`""` is the main thread's
/// shutdown write), each repeated as many times as it was recorded.
fn oracle_trips<'a>(oracle: &'a Value, keep: &dyn Fn(&Value) -> bool) -> Vec<&'a Value> {
    oracle["round_trips"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| keep(t))
        .flat_map(|t| std::iter::repeat_n(t, t["times"].as_u64().unwrap_or(1) as usize))
        .collect()
}

/// The top-level keys of a JSON object, in the order they appear in the
/// text (serde_json's map sorts them).
fn top_level_keys(raw: &str) -> Vec<String> {
    let bytes = raw.as_bytes();
    let mut keys = Vec::new();
    let mut depth = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let mut j = i + 1;
                while bytes[j] != b'"' {
                    j += if bytes[j] == b'\\' { 2 } else { 1 };
                }
                if depth == 1 && bytes.get(j + 1) == Some(&b':') {
                    keys.push(serde_json::from_str::<String>(&raw[i..=j]).unwrap());
                }
                i = j;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    keys
}

/// A driver argument with the oracle's masks: the identity, and a stats key's date.
fn mask(arg: &str) -> String {
    let arg = arg.replace(IDENTITY, "<identity>");
    for prefix in ["stat:processed:", "stat:failed:"] {
        if let Some(date) = arg.strip_prefix(prefix) {
            let b = date.as_bytes();
            assert!(
                b.len() == 10 && b[4] == b'-' && b[7] == b'-' && date.chars().filter(char::is_ascii_digit).count() == 8,
                "a stats key's date is YYYY-MM-DD: {arg}"
            );
            return format!("{prefix}<YYYY-MM-DD>");
        }
    }
    arg
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

/// `info` as the driver wrote it, against Sidekiq's: the same keys in
/// the same order, the same values once the per-run ones are masked,
/// and `labels` Sidekiq's plus `"roundhouse"`.
fn assert_info(raw: &str, want: &Value) {
    assert_eq!(top_level_keys(raw), INFO_KEYS, "info's key order differs from Sidekiq's: {raw}");
    let mut got: Value = serde_json::from_str(raw).unwrap_or_else(|e| panic!("info is not JSON ({e}): {raw}"));
    assert_eq!(got["hostname"], "host-1", "{raw}");
    assert_eq!(got["pid"], 4242, "{raw}");
    assert_eq!(got["identity"], IDENTITY, "{raw}");
    assert_eq!(got["started_at"].as_f64(), Some(STARTED_AT), "{raw}");
    let mut labels = want["labels"].as_array().unwrap().clone();
    labels.push("roundhouse".into());
    assert_eq!(got["labels"], Value::Array(labels), "info's labels are Sidekiq's plus roundhouse: {raw}");
    got["hostname"] = "<hostname>".into();
    got["pid"] = "<pid>".into();
    got["identity"] = "<identity>".into();
    got["started_at"] = "<epoch Float>".into();
    got["labels"] = want["labels"].clone();
    assert_eq!(&got, want, "info differs from Sidekiq's: {raw}");
}

/// `HSET <identity> info … rss …`: Sidekiq's fields in its order.
fn assert_process_hash(args: &[String], want: &Value, beat: f64) {
    let pairs: Vec<(&str, &str)> = args.chunks(2).map(|p| (p[0].as_str(), p[1].as_str())).collect();
    let order: Vec<&str> = pairs.iter().map(|p| p.0).collect();
    assert_eq!(order, strs(&want["field_order"]), "the process hash's fields differ from Sidekiq's");
    for (field, value) in pairs {
        let w = &want["fields"][field];
        match field {
            "info" => assert_info(value, w),
            "beat" => {
                assert!(value.contains('.') && value.parse::<f64>().is_ok(), "beat is a Float's to_s: {value}");
                assert_eq!(value.parse::<f64>().unwrap(), beat, "beat is the scripted clock's");
            }
            // The recording client checks these for digits.
            "rtt_us" | "rss" => assert_eq!(value, "<int>", "{field} is an Integer"),
            _ => assert_eq!(value, w.as_str().unwrap(), "{field} differs from Sidekiq's"),
        }
    }
}

/// `HSET <identity>:work <tid> <entry> …`: one entry per running job.
fn assert_work_hash(args: &[String], want: &Value) {
    let want = want.as_array().unwrap();
    assert_eq!(args.len(), want.len() * 2, "work entries: {args:?}");
    for (pair, w) in args.chunks(2).zip(want) {
        let w = &w["value"];
        assert!(!pair[0].is_empty() && pair[0].bytes().all(|b| b.is_ascii_digit() || b.is_ascii_lowercase()), "a tid is base 36: {}", pair[0]);
        assert_eq!(top_level_keys(&pair[1]), strs(&w["keys"]), "a work entry's keys differ from Sidekiq's: {}", pair[1]);
        let entry: Value = serde_json::from_str(&pair[1]).unwrap();
        assert_eq!(entry["queue"], w["queue"], "{}", pair[1]);
        let mut payload: Value = serde_json::from_str(entry["payload"].as_str().expect("payload is the job's text")).unwrap();
        payload["jid"] = "<jid>".into();
        payload["created_at"] = "<ms>".into();
        payload["enqueued_at"] = "<ms>".into();
        assert_eq!(payload, w["payload"], "the running job differs from Sidekiq's: {}", pair[1]);
        let run_at = entry["run_at"].as_i64().expect("run_at is an Integer");
        assert_eq!(run_at.to_string().len(), 10, "run_at is epoch seconds: {run_at}");
    }
}

/// One command against Sidekiq's. `beat` is the trip's scripted time.
fn assert_command(got: &[String], want: &Value, beat: f64) {
    let argv = strs(&want["argv"]);
    assert_eq!(got[0].to_lowercase(), argv[0].to_lowercase(), "command differs from Sidekiq's: {got:?} against {argv:?}");
    match got[0].as_str() {
        "HSET" if got[1] == IDENTITY => assert_process_hash(&got[2..], want, beat),
        "HSET" => {
            assert_eq!(mask(&got[1]), argv[1], "{got:?}");
            assert_work_hash(&got[2..], &want["entries"]);
        }
        // SET's options in either order: spinel-redis writes EX before NX.
        "SET" => {
            assert_eq!(got[1..3], argv[1..3], "{got:?}");
            let options = |a: &[String]| {
                let mut o: Vec<String> = Vec::new();
                let mut i = 0;
                while i < a.len() {
                    if a[i].eq_ignore_ascii_case("ex") {
                        o.push(format!("ex {}", a[i + 1]));
                        i += 2;
                    } else {
                        o.push(a[i].to_lowercase());
                        i += 1;
                    }
                }
                o.sort();
                o
            };
            assert_eq!(options(&got[3..]), options(&argv[3..]), "SET options differ from Sidekiq's: {got:?}");
        }
        _ => {
            let masked: Vec<String> = got[1..].iter().map(|a| mask(a)).collect();
            assert_eq!(masked, argv[1..], "command differs from Sidekiq's: {got:?}");
        }
    }
}

fn assert_trip(got: &Trip, want: &Value, beat: f64) {
    let commands = want["commands"].as_array().unwrap();
    assert_eq!(got.kind, want["kind"].as_str().unwrap(), "round trip kind: {:?}", got.commands);
    assert_eq!(got.commands.len(), commands.len(), "round trip commands: {:?} against {commands:?}", got.commands);
    for (g, w) in got.commands.iter().zip(commands) {
        assert_command(g, w, beat);
    }
}

/// The commands of round trips that Sidekiq pipelines and this client
/// sends one at a time: the same commands, each its own `call`.
fn assert_unpipelined(got: &[Trip], want: &[&Value]) {
    assert!(want.iter().all(|w| w["kind"] == "pipelined" || w["kind"] == "call"), "{want:?}");
    let want_commands: Vec<&Value> = want.iter().flat_map(|w| w["commands"].as_array().unwrap()).collect();
    assert!(got.iter().all(|t| t.kind == "call" && t.commands.len() == 1), "one command per round trip");
    assert_eq!(got.len(), want_commands.len(), "commands: {:?}", got.iter().map(|t| &t.commands).collect::<Vec<_>>());
    for (g, w) in got.iter().zip(want_commands) {
        assert_command(&g.commands[0], w, 0.0);
    }
}

fn assert_matches_sidekiq(stdout: &str) {
    let oracle = oracle();
    // The heartbeat thread's round trips, beat by beat.
    let want = oracle_trips(&oracle, &|t| t["thread"] == "sidekiq.heartbeat");
    let got = driver_trips(stdout, "heartbeat");
    assert_eq!(got.len(), want.len(), "the heartbeat's round trips");
    let mut beats = 0.0;
    for (g, w) in got.iter().zip(&want) {
        if g.commands.iter().any(|c| c[0] == "SADD") {
            assert_trip(g, w, FIRST_BEAT + 10.0 * beats);
            beats += 1.0;
        } else {
            assert_trip(g, w, 0.0);
        }
    }
    // The poller's cleanup: everything it sent but its SCARDs.
    let want = oracle_trips(&oracle, &|t| {
        t["thread"] == "sidekiq.scheduler" && t["commands"][0]["argv"][0].as_str().is_some_and(|c| !c.eq_ignore_ascii_case("scard"))
    });
    assert_unpipelined(&driver_trips(stdout, "cleanup"), &want);
    // The shutdown write.
    let want = oracle_trips(&oracle, &|t| t["thread"] == "");
    assert_unpipelined(&driver_trips(stdout, "shutdown"), &want);
}

#[test]
fn the_heartbeat_writes_what_sidekiq_writes() {
    let stdout = run_cruby(&scratch("roundhouse-sidekiq-heartbeat-cruby"));
    assert_matches_sidekiq(&stdout);
}

#[test]
#[ignore]
fn the_spinel_heartbeat_writes_what_cruby_writes() {
    let cruby = run_cruby(&scratch("roundhouse-sidekiq-heartbeat-cruby-for-spinel"));
    let dir = scratch("roundhouse-sidekiq-heartbeat-spinel");
    let compiler = std::env::var("SPINEL").unwrap_or_else(|_| "spinel".into());
    let build = Command::new(compiler)
        .args(["-I", ".", "driver.rb", "-o", "driver"])
        .current_dir(&dir)
        .output()
        .expect("spinel is on PATH");
    assert!(
        build.status.success(),
        "spinel failed to compile driver.rb\n=== stdout ===\n{}\n=== stderr ===\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(dir.join("driver")).current_dir(&dir).output().unwrap();
    let binary = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(binary, cruby, "the binary and CRuby disagree\n=== spinel ===\n{binary}\n=== CRuby ===\n{cruby}");
}
