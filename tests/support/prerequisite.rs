//! A missing prerequisite: the one way a test that needs more than Rust
//! (a gem for a native Rails control, a Ruby on PATH, a native compiler)
//! says it did not run.
//!
//! A test that skips passes, and nextest keeps a passing test's output to
//! itself, so a skip printed with `eprintln!` reaches neither the CI log
//! nor the JUnit report: the test reads as run. `missing(what)` keeps the
//! plain `cargo test` that needs only Rust, and makes a skip visible:
//!
//! - `ROUNDHOUSE_SKIP_LOG=<file>` appends `<test>\t<what>` to that file,
//!   which CI publishes in the job summary.
//! - `ROUNDHOUSE_REQUIRE_PREREQUISITES=1` fails the test as well, for a
//!   run that provides everything and so must not skip anything; the log
//!   still names it.
//!
//! The caller returns after it, as it did after its `eprintln!`.
//! PostgreSQL has its own pair (`pg_test_url.rs`), which predates this.
#![allow(dead_code)]

use std::io::Write;

pub const LOG_VAR: &str = "ROUNDHOUSE_SKIP_LOG";
pub const REQUIRE_VAR: &str = "ROUNDHOUSE_REQUIRE_PREREQUISITES";

/// Report that this test's prerequisite `what` is absent, in the skip log,
/// then as a failure when the run requires prerequisites, otherwise as a
/// skip on stderr.
pub fn missing(what: &str) {
    let current = std::thread::current();
    let test = current.name().unwrap_or("<unnamed test>");
    if let Some(log) = std::env::var_os(LOG_VAR) {
        // One write of one line: concurrent test processes append whole lines.
        let line = format!("{test}\t{}\n", what.replace(['\t', '\n'], " "));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .unwrap_or_else(|e| panic!("{LOG_VAR}={}: {e}", log.to_string_lossy()));
        file.write_all(line.as_bytes()).expect("append to the skip log");
    }
    assert!(
        !std::env::var(REQUIRE_VAR).is_ok_and(|v| !v.is_empty()),
        "{test}: {what}; {REQUIRE_VAR} is set, so a missing prerequisite fails instead of skipping"
    );
    eprintln!("skipping {test}: {what}");
}

/// `command`'s output, or `None` once `missing(...)` has said that its
/// program is not on `PATH`: the caller returns. Any other error starting
/// it fails the test, and what the program answered is the caller's to judge.
pub fn output_of(command: &mut std::process::Command) -> Option<std::process::Output> {
    match command.output() {
        Ok(output) => Some(output),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A working directory that is not there fails the same way, and is no missing program.
            if let Some(dir) = command.get_current_dir() {
                assert!(dir.is_dir(), "{command:?}: its directory {} is not there", dir.display());
            }
            missing(&format!("{} not available on PATH", command.get_program().to_string_lossy()));
            None
        }
        Err(error) => panic!("{command:?}: {error}"),
    }
}
