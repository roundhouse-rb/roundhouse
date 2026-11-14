use super::view_component_gem::{native_app, KNOWN_GAPS, NATIVE_EXPECTED, NATIVE_SCRIPT};

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn a_component_renders_its_template_natively() {
    let Some(app) = native_app() else { return };
    // A marker before the render and the backtrace of what it raises: a native binary prints only the message.
    let script = format!("puts \"booted\"\nbegin\n{NATIVE_SCRIPT}rescue => e\n  puts e.class.name\n  puts e.message\n  puts e.backtrace.to_a.first(15)\n  raise\nend\n");
    let run = app.run_spinel(&script);
    let unknown: Vec<_> = run.errors.iter().filter(|e| !KNOWN_GAPS.iter().any(|gap| e.contains(gap))).collect();
    assert!(run.program_succeeded() && unknown.is_empty(), "{unknown:#?}\n{}\n{}\n{}\n{}", run.status, run.emitted.display(), run.stdout, run.stderr);
    assert_eq!(run.stdout, format!("booted\n{NATIVE_EXPECTED}"));
}
