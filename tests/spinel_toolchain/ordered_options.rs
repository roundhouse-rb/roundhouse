use super::ordered_options_app::{app, EXPECTED, SCRIPT};

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn an_ordered_options_key_reads_through_its_name_natively() {
    let run = app().run_spinel_slim(&["runtime/ordered_options_spinel"], SCRIPT);
    run.assert_passes();
    assert_eq!(run.stdout, EXPECTED);
}
