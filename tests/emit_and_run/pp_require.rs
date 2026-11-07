//! A model that requires `pp` and pretty-prints a value, the form a
//! debugging helper writes. The require used to be left out of the spinel
//! tree's boot.rb (`SPINEL_LACKS`), because a `require "pp"` that no call
//! followed was refused by Spinel's layout check; matz/spinel#11262 takes
//! the refusal away, and the require now reaches both trees.

use super::emit_and_run;

const PROBE: &str = r#"require "pp"

class PrettyProbe
  def self.show(value)
    pp(value)
  end
end
"#;

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog().write("app/models/pretty_probe.rb", PROBE)
}

const ASSERTIONS: &str = r##"got = PrettyProbe.show({a: 1, b: [1, 2]})
raise got.inspect unless got == {a: 1, b: [1, 2]}
puts "pp require passed"
"##;

#[test]
fn pp_require_runs() {
    let run = app().run_ruby(ASSERTIONS);
    run.assert_passes();
    assert!(run.stdout.contains("{a: 1, b: [1, 2]}\npp require passed"), "{}", run.stdout);
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn pp_require_runs_on_spinel() {
    let run = app().run_spinel(ASSERTIONS);
    run.assert_passes();
    assert!(run.stdout.contains("{a: 1, b: [1, 2]}\npp require passed"), "{}", run.stdout);
}

/// The shape that was refused: a model that requires `pp` and nothing in the
/// program pretty-prints. PP is then a class nothing names, and Spinel before
/// matz/spinel#11262 laid its inherited `@group_stack` out as a poly array
/// under PrettyPrint's object array.
const QUIET: &str = r#"require "pp"

class QuietProbe
  def self.size = 1
end
"#;

#[test]
#[ignore = "requires the Spinel toolchain"]
fn pp_require_without_a_call_compiles_on_spinel() {
    let run = emit_and_run::real_blog()
        .write("app/models/quiet_probe.rb", QUIET)
        .run_spinel("puts QuietProbe.size\n");
    run.assert_passes();
    assert!(run.stdout.contains("1\n"), "{}", run.stdout);
}
