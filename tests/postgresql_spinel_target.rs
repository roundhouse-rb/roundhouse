//! PostgreSQL on the Spinel target: an app whose `config/database.yml`
//! names postgresql for the test environment's primary gets SQL lowered
//! for PostgreSQL, with the read binds off as on the Ruby target, and a
//! tree that runs it on spinel-pg (`project::apply_postgresql_tree`):
//! runtime/spinel/db_pg.rb as its Db over spinel-pg's driver, `pg` in
//! spin.toml, and its database the server `DATABASE_URL` names, one
//! schema per test process.
//!
//! These check the emitted tree and need neither Spinel nor a server.
//! The native run against PostgreSQL is in tests/spinel_pg_db.rs, where
//! CI provides both.

#[path = "support/emit_and_run.rs"]
mod emit_and_run;

use roundhouse::project::BuildTarget;

const POSTGRESQL_DATABASE_YML: &str = "default: &default\n  adapter: postgresql\n  encoding: unicode\n  max_connections: <%= ENV.fetch(\"RAILS_MAX_THREADS\") { 5 } %>\n\ndevelopment:\n  <<: *default\n  database: blog_development\n\ntest:\n  <<: *default\n  database: blog_test\n";

fn postgresql_blog() -> emit_and_run::Overlay {
    emit_and_run::real_blog().write("config/database.yml", POSTGRESQL_DATABASE_YML)
}

fn read(emitted: &std::path::Path, path: &str) -> String {
    std::fs::read_to_string(emitted.join(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

#[test]
fn a_postgresql_primary_ships_the_spinel_pg_tree() {
    let (emitted, errors) = postgresql_blog().emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    let db = read(&emitted, "runtime/db.rb");
    assert!(db.contains("Primitive Db surface over PostgreSQL"), "runtime/db.rb is not db_pg.rb");
    assert!(!emitted.join("runtime/db_pg.rb").exists(), "db_pg.rb is runtime/db.rb, not beside it");
    let driver = read(&emitted, "runtime/pg_driver.rb");
    assert!(driver.contains("PG.connect(host, port, database, user, password)"), "runtime/pg_driver.rb is not spinel-pg's:\n{driver}");

    // spinel-pg as a spin dependency, pinned to the release that sends
    // PGOPTIONS, which the test helper's schema rides.
    let manifest = read(&emitted, "spin.toml");
    assert!(
        manifest.contains("pg = { git = \"https://github.com/roundhouse-rb/spinel-pg\", ref = \"v0.2.1\" }"),
        "{manifest}"
    );
    let gemfile = read(&emitted, "Gemfile");
    assert!(gemfile.contains("gem \"pg\"") && !gemfile.contains("gem \"sqlite3\""), "{gemfile}");
    assert!(!emitted.join("Gemfile.lock").exists(), "the committed lock is SQLite's");

    // The server opens the database DATABASE_URL names, and says so.
    let main = read(&emitted, "main.rb");
    assert!(main.contains("    path = ENV.fetch(\"DATABASE_URL\", \"\")\n"), "{main}");
    assert!(!main.contains("    path = Main.default_db_path\n"), "the server still opens the SQLite file");
    assert!(main.contains("PostgreSQL: DATABASE_URL"), "--help still names the SQLite file");

    // Each test process works in a schema of its own, which every pooled
    // connection reaches through PGOPTIONS.
    let helper = read(&emitted, "test/test_helper.rb");
    assert!(!helper.contains("Db.configure(_test_db_path)\n"), "{helper}");
    assert!(helper.contains("ENV[\"PGOPTIONS\"] = \"-c search_path=\" + TEST_SCHEMA\n"), "{helper}");
    assert!(helper.contains("at_exit { Db.exec(\"DROP SCHEMA IF EXISTS \" + TEST_SCHEMA + \" CASCADE\") }"), "{helper}");
}

/// The SQL in the tree is PostgreSQL's: the boot DDL, a truncate that
/// resets the identity in the statement itself, and no read binds, whose
/// `?` marks spinel-pg's `$n` placeholders do not read.
#[test]
fn a_postgresql_primary_lowers_its_sql_for_postgresql_on_spinel() {
    let (emitted, errors) = postgresql_blog().emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    let schema = read(&emitted, "config/schema.rb");
    assert!(schema.contains("\\\"id\\\" bigserial PRIMARY KEY"), "{schema}");
    let article = read(&emitted, "app/models/article.rb");
    assert!(article.contains("TRUNCATE TABLE \\\"articles\\\" RESTART IDENTITY CASCADE"), "{article}");
    assert!(!article.contains("sqlite_sequence"), "{article}");
    assert!(!article.contains("Db.bind_"), "a read binds a `?` value:\n{article}");
}

#[test]
fn a_sqlite_primary_keeps_the_sqlite_spinel_tree() {
    let (emitted, errors) = emit_and_run::real_blog().emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    assert!(read(&emitted, "runtime/db.rb").contains("SQLite-via-libsqlite3-FFI"));
    assert!(!read(&emitted, "spin.toml").contains("spinel-pg"));
    assert!(read(&emitted, "Gemfile").contains("gem \"sqlite3\""));
    assert!(read(&emitted, "main.rb").contains("    path = Main.default_db_path\n"));
    assert!(read(&emitted, "test/test_helper.rb").contains("Db.configure(_test_db_path)\n"));
    let article = read(&emitted, "app/models/article.rb");
    assert!(article.contains("sqlite_sequence") && article.contains("Db.bind_"), "{article}");
}

#[test]
fn sqlite_functions_on_a_postgresql_primary_are_refused_on_spinel() {
    let (_emitted, errors) = postgresql_blog()
        .write("config/initializers/sqlite.rb", "conn.create_function(\"regexp\", 2) do |fn, pattern, expr|\n  fn.result = 1\nend\n")
        .emit(BuildTarget::Spinel);
    assert!(errors.iter().any(|e| e.contains("SQLite functions (regexp)") && e.contains("postgresql")), "{errors:#?}");
}
