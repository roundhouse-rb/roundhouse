use std::path::Path;
use std::process::Command;

fn ruby_driver(name: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("ruby")
        .arg(root.join(format!("tests/{name}.rb")))
        .output()
        .expect("ruby is on PATH");
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("done") || stdout.contains("skip:"),
        "{stdout}"
    );
}

#[test]
fn wrapper_setup_failure_does_not_escape() {
    ruby_driver("spinel_chunked_body_io");
}

#[test]
fn chunked_framing_across_all_servers() {
    ruby_driver("spinel_chunked_body");
}

#[test]
fn puma_confirms_the_chunked_body_contract() {
    ruby_driver("spinel_chunked_body_puma");
}

#[test]
#[ignore = "deterministic CRuby slice-work contract; run in the framework lane"]
fn chunk_count_has_linear_cost() {
    ruby_driver("spinel_chunked_body_work");
}

#[test]
#[ignore = "requires native Spinel; set SPINEL to its executable"]
fn chunked_body_on_spinel() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::temp_dir().join(format!("roundhouse-chunked-body-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let binary = dir.join("server");
    let compiled = Command::new(std::env::var("SPINEL").unwrap_or_else(|_| "spinel".into()))
        .arg(root.join("tests/spinel_chunked_body_native.rb"))
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("compile native server");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let out = Command::new("ruby")
        .arg(root.join("tests/spinel_chunked_body_native_check.rb"))
        .arg(&binary)
        .output()
        .expect("drive native server");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
