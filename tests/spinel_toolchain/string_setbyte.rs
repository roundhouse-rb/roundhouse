//! `String#setbyte` compiled natively; the CRuby twin is
//! `emit_and_run/string_setbyte.rs`.

use super::string_setbyte_contract as contract;

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn setbyte_writes_the_byte_in_place_natively() {
    let run = contract::overlay().run_spinel_slim(&[], contract::SCRIPT);
    run.assert_passes();
    assert_eq!(run.stdout, contract::EXPECTED, "stderr:\n{}", run.stderr);
}
