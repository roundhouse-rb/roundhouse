use super::{emit_and_run, option_dsl};

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn an_extended_option_dsl_runs_natively() {
    let run = emit_and_run::real_blog()
        .write("app/models/settings/option.rb", option_dsl::OPTION)
        .write("app/models/settings/config.rb", &option_dsl::config(option_dsl::LITERAL_OPTIONS, ""))
        .run_spinel_slim(&[], &format!("require_relative \"app/models/settings\"\n{}", option_dsl::LITERAL_ASSERTIONS));
    run.assert_passes();
    assert!(run.stdout.contains("option DSL passed"), "{}", run.stdout);
}
