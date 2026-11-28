//! An app that routes its models by role and shard, compiled natively
//! over two SQLite files on disk: each report printed by the binary is
//! Rails 8.1.4's for the same statements (`support/database_routes.rs`).

use super::database_routes_contract as contract;

/// `connects_to shards:`: the two-file reduction, nested shard and role
/// blocks, restoration after a raise, a `return` and a `break`, the
/// write guard on every mutation, lazy missing pools, two threads on
/// opposite shards at once, and one lease across both files.
#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn shards_route_to_their_own_files_natively() {
    let run = contract::app(contract::SHARDS, &contract::shards_report())
        .run_spinel(&format!("{}{}", contract::BOOT, contract::SHARDS_SCRIPT));
    run.assert_passes();
    let expected = [
        contract::TWO_FILES_EXPECTED,
        contract::SHARDS_EXPECTED,
        contract::ERRORS_EXPECTED,
        contract::EXITS_EXPECTED,
        contract::GUARDS_EXPECTED,
        contract::ISOLATION_EXPECTED,
        contract::HANDLES_EXPECTED,
        contract::LEASE_EXPECTED,
    ];
    let sections: Vec<_> = run.stdout.split("--\n").collect();
    assert_eq!(sections.len(), expected.len(), "{}", run.stdout);
    for (index, expected) in expected.iter().enumerate() {
        assert_eq!(
            contract::comparable(sections[index]),
            contract::comparable(expected),
            "section {index}\n{}\n{}",
            run.stdout,
            run.stderr
        );
    }
}

/// `connects_to database:` with a reading role: the write guard's
/// nesting, and a writing block on ActiveRecord::Base inside a reading
/// one, which ingest used to read as its body.
#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn roles_guard_writes_natively() {
    // The app's own class methods win over Rails' readers of the same
    // name, as in Ruby: one the connection class defines, and one from a
    // module a model extends. The report calls neither.
    let record = contract::roles_plain().replace("\nend", "\n\n  def self.default_shard\n    \"custom\"\n  end\nend");
    let run = contract::app(&record, &contract::roles_report())
        .write("app/models/titles.rb", "module Titles\n  def current_role\n    \"admin\"\n  end\nend\n")
        .write("app/models/note.rb", "class Note < ApplicationRecord\n  extend Titles\nend\n")
        .run_spinel_with_rbs(&format!(
            "{}RouteReport.roles\nputs Note.default_shard + \"!\"\nputs Note.current_role + \"!\"\n{}",
            contract::BOOT,
            contract::ONE_CONNECTION_SCRIPT
        ));
    run.assert_passes();
    let expected = format!("{}custom!\nadmin!\n{}", contract::ROLES_EXPECTED, contract::ONE_CONNECTION_EXPECTED);
    assert_eq!(contract::comparable(&run.stdout), contract::comparable(&expected), "{}", run.stderr);
}

fn assert_shard_report(script: &str, expected: &str) {
    let run = contract::app(contract::SHARDS, &contract::shards_report())
        .run_spinel(&format!("{}{script}", contract::BOOT));
    run.assert_passes();
    assert_eq!(contract::comparable(&run.stdout), contract::comparable(expected), "{}", run.stderr);
}

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn a_held_base_connection_preserves_model_routing() {
    assert_shard_report(contract::HELD_ROUTE_SCRIPT, contract::HELD_ROUTE_EXPECTED);
}

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn raw_transactions_refuse_cross_slot_switches() {
    assert_shard_report(contract::RAW_SWITCH_SCRIPT, contract::RAW_SWITCH_EXPECTED);
}

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn a_same_file_write_refreshes_the_held_reader_snapshot() {
    assert_shard_report(contract::SNAPSHOT_SCRIPT, contract::SNAPSHOT_EXPECTED);
}

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn writable_routes_create_missing_parent_directories() {
    let yml = contract::database_yml().replace("storage/right.sqlite3", "storage/missing/deep/right.sqlite3")
        .replace("left_reader:\n    adapter: sqlite3\n    database: storage/left.sqlite3", "left_reader:\n    adapter: sqlite3\n    database: storage/missing/deep/right.sqlite3");
    let run = contract::app(contract::SHARDS, &contract::shards_report())
        .write("config/database.yml", &yml)
        .run_spinel(&format!("{}ApplicationRecord.connected_to(shard: :right) {{ puts Note.count }}\nputs File.exist?(\"storage/missing/deep/right.sqlite3\")", contract::BOOT));
    run.assert_passes();
    assert_eq!(run.stdout, "0\ntrue\n");
}

/// `Db.configure` opens each route's file from boot.rb's resolver by its
/// root name, so an app's own `Db::AppDatabase` does not redirect it: the
/// write lands in the shard's file, as in Rails.
#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn an_apps_own_db_app_database_does_not_redirect_a_route() {
    let decoy = "module Db\n  module AppDatabase\n    def self.path_for_name(env, name)\n      \"storage/decoy.sqlite3\"\n    end\n  end\nend\n";
    let run = contract::app(contract::SHARDS, &contract::shards_report())
        .write("app/models/db/app_database.rb", decoy)
        .run_spinel(&format!(
            "{}ApplicationRecord.connected_to(shard: :right) {{ Note.create!(tag: \"r\") }}\nApplicationRecord.connected_to(shard: :right) {{ puts Note.count }}\nputs File.exist?(\"storage/decoy.sqlite3\")",
            contract::BOOT
        ));
    run.assert_passes();
    assert_eq!(run.stdout, "1\nfalse\n", "{}", run.stderr);
}

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn writable_route_parents_tolerate_concurrent_creators() {
    assert_shard_report(contract::DIRECTORY_RACE_SCRIPT, contract::DIRECTORY_RACE_EXPECTED);
}

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn a_closed_parked_connection_is_not_reused_after_reconfigure() {
    assert_shard_report(contract::PARKED_REOPEN_SCRIPT, contract::PARKED_REOPEN_EXPECTED);
}

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn a_shard_scope_does_not_restore_a_closed_outer_connection() {
    assert_shard_report(contract::SCOPED_REOPEN_SCRIPT, contract::SCOPED_REOPEN_EXPECTED);
}

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn a_scope_parameter_default_keeps_its_writing_scope() {
    let note = "class Note < ApplicationRecord\n  scope :chosen, ->(row = ActiveRecord::Base.connected_to(role: :writing) { Note.create!(tag: 'default').id }) { where(id: row) }\nend\n";
    let run = contract::app(contract::SHARDS, &contract::shards_report()).write("app/models/note.rb", note)
        .run_spinel(&format!("{}ApplicationRecord.connected_to(role: :reading) {{ puts Note.chosen.first.tag }}", contract::BOOT));
    run.assert_passes();
    assert_eq!(run.stdout, "default\n");
}

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn the_declared_default_shard_is_available_before_configuration() {
    let run = contract::app(contract::SHARDS, &contract::shards_report())
        .run_spinel(&format!("before = ApplicationRecord.default_shard\ncurrent = ApplicationRecord.current_shard\n{}puts before.to_s\nputs current.to_s", contract::BOOT));
    run.assert_passes();
    assert_eq!(run.stdout, "left\nleft\n");
}

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn a_write_skips_closed_held_snapshots_after_reconfigure() {
    assert_shard_report(contract::REFRESH_REOPEN_SCRIPT, contract::REFRESH_REOPEN_EXPECTED);
}

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn a_shared_named_schema_supports_a_primary_default_writer() {
    let yml = contract::database_yml().replace("database: storage/primary.sqlite3\n", "database: storage/primary.sqlite3\n    schema_dump: catalog_schema.rb\n")
        .replace("schema_dump: schema.rb", "schema_dump: catalog_schema.rb");
    let record = contract::SHARDS.replace("left: { writing: :left,", "left: { writing: :primary,");
    let schema = "ActiveRecord::Schema[8.1].define(version: 1) do\n  create_table :notes do |t|\n    t.string :tag\n  end\nend\n";
    let run = contract::app(&record, "class RouteReport\nend\n").remove("db/schema.rb")
        .write("db/catalog_schema.rb", schema).write("config/database.yml", &yml)
        .run_spinel(&format!("{}Note.create!(tag: \"primary\")\nApplicationRecord.connected_to(shard: :right) {{ Note.create!(tag: \"secondary\"); puts Note.first.tag }}\nputs Note.first.tag", contract::BOOT));
    run.assert_passes();
    assert_eq!(run.stdout, "secondary\nprimary\n");
}

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn association_scopes_and_defaults_keep_their_writing_scope() {
    let schema = "ActiveRecord::Schema[8.1].define(version: 1) do\n  create_table :notes do |t|\n    t.string :tag, null: false\n    t.integer :parent_id\n  end\nend\n";
    let note = "class Note < ApplicationRecord\n  has_many :children, -> { where(tag: ActiveRecord::Base.connected_to(role: :writing) { ApplicationRecord.current_role.to_s }) }, class_name: 'Note', foreign_key: :parent_id\n  belongs_to :parent, class_name: 'Note', optional: true, default: -> { ActiveRecord::Base.connected_to(role: :writing) { Note.find_by(tag: ApplicationRecord.current_role.to_s) } }\nend\n";
    let run = contract::app(contract::SHARDS, "class RouteReport\nend\n").write("db/schema.rb", schema).write("app/models/note.rb", note)
        .run_spinel(&format!("{}owner = Note.create!(tag: \"owner\")\nNote.create!(tag: \"writing\", parent_id: owner.id)\nApplicationRecord.connected_to(role: :reading) do\n  puts \"association=\" + owner.children.count.to_s\n  fresh = Note.new(tag: \"default\")\n  fresh.valid?\n  puts \"default=\" + fresh.parent_id.to_s\nend\n", contract::BOOT));
    run.assert_passes();
    assert_eq!(run.stdout, "association=1\ndefault=2\n");
}
