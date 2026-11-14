use super::ordered_options_app::{app, EXPECTED, SCRIPT};

#[test]
fn an_ordered_options_key_reads_through_its_name_on_ruby() {
    let run = app().run_ruby(SCRIPT);
    run.assert_passes();
    assert_eq!(run.stdout, EXPECTED);
}

