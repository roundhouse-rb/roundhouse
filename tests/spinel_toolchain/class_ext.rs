use super::class_ext_app::{app, EXPECTED, SCRIPT};

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn descendants_and_module_parents_answer_as_rails_does_natively() {
    let run = app().run_spinel_slim(&["runtime/class_ext_spinel"], SCRIPT);
    run.assert_passes();
    assert_eq!(run.stdout, EXPECTED);
}
