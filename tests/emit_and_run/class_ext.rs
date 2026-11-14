use super::class_ext_app::{app, EXPECTED, SCRIPT};

#[test]
fn descendants_and_module_parents_answer_as_rails_does_on_ruby() {
    let run = app().run_ruby(SCRIPT);
    run.assert_passes();
    assert_eq!(run.stdout, EXPECTED);
}
