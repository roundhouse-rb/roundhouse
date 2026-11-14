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

    // spinel-pg as a spin dependency, pinned to its release with the
    // extended query protocol.
    let manifest = read(&emitted, "spin.toml");
    assert!(
        manifest.contains("pg = { git = \"https://github.com/roundhouse-rb/spinel-pg\", ref = \"v0.2.0\" }"),
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
    // connection reaches as its schema_search_path.
    let helper = read(&emitted, "test/test_helper.rb");
    assert!(!helper.contains("Db.configure(_test_db_path)\n"), "{helper}");
    assert!(helper.contains("Db.configure(ENV.fetch(\"DATABASE_URL\", \"\"), schema_search_path: TEST_SCHEMA)\n"), "{helper}");
    assert!(!helper.contains("PGOPTIONS"), "{helper}");
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

/// The common deployed shape, a literal adapter beside a computed `url:`
/// (#2011), selects the same tree.
#[test]
fn a_literal_adapter_beside_a_computed_url_selects_the_spinel_pg_tree() {
    let (emitted, errors) = emit_and_run::real_blog()
        .write(
            "config/database.yml",
            "default: &default\n  adapter: postgresql\n  url: <%= ENV[\"DATABASE_URL\"] %>\n\ntest:\n  <<: *default\n\nproduction:\n  <<: *default\n",
        )
        .emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    assert!(read(&emitted, "runtime/db.rb").contains("Primitive Db surface over PostgreSQL"));
    assert!(read(&emitted, "spin.toml").contains("roundhouse-rb/spinel-pg"));
}

/// A table with no primary key (`id: false`, a join table) has no
/// sequence for PostgreSQL to read an inserted key from, and Rails reads
/// none: the record's id stays nil. The insert writes the row and reads
/// no key, on both targets that run PostgreSQL. SQLite keeps its rowid read.
#[test]
fn an_insert_into_a_keyless_table_reads_no_key_on_postgresql() {
    let keyless = |database_yml: &str| {
        emit_and_run::real_blog()
            .write("config/database.yml", database_yml)
            .edit(
                "db/schema.rb",
                "  create_table \"comments\"",
                "  create_table \"stars\", id: false, force: :cascade do |t|\n    t.bigint \"article_id\", null: false\n  end\n\n  create_table \"comments\"",
            )
            .write("app/models/star.rb", "class Star < ApplicationRecord\nend\n")
    };
    let insert = |model: &str| {
        let start = model.find("def _adapter_insert").expect("_adapter_insert");
        let end = model[start..].find("\n  end\n").map(|at| start + at).expect("end of _adapter_insert");
        model[start..end].to_string()
    };
    for target in [BuildTarget::Spinel, BuildTarget::Ruby] {
        let (emitted, errors) = keyless(POSTGRESQL_DATABASE_YML).emit(target);
        assert!(errors.is_empty(), "{target:?}: {errors:#?}");
        let body = insert(&read(&emitted, "app/models/star.rb"));
        assert!(body.contains("INSERT INTO stars") && !body.contains("last_insert_rowid"), "{target:?}:\n{body}");
        // A keyed table still reads its key back.
        assert!(insert(&read(&emitted, "app/models/article.rb")).contains("last_insert_rowid"), "{target:?}");
    }
    let (emitted, errors) = keyless("test:\n  adapter: sqlite3\n  database: \":memory:\"\n").emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    assert!(insert(&read(&emitted, "app/models/star.rb")).contains("last_insert_rowid"));
}

/// The same keyless tables with a stored generated column or a database
/// default: the insert reads those back with RETURNING, as Rails reads
/// them, and on PostgreSQL returns no `rowid` (which it has no column
/// for) in the key's place. SQLite keeps its `CAST(rowid AS TEXT)`.
#[test]
fn a_keyless_insert_returns_its_generated_and_default_columns_without_rowid_on_postgresql() {
    let app = |database_yml: &str| {
        emit_and_run::real_blog()
            .write("config/database.yml", database_yml)
            .edit(
                "db/schema.rb",
                "  create_table \"comments\"",
                "  create_table \"badges\", id: false, force: :cascade do |t|\n    t.string \"first_name\", null: false\n    t.virtual \"label\", type: :string, as: \"first_name || '!'\", stored: true\n  end\n\n  create_table \"visits\", id: false, force: :cascade do |t|\n    t.string \"name\"\n    t.datetime \"recorded_at\", default: -> { \"CURRENT_TIMESTAMP\" }, null: false\n  end\n\n  create_table \"comments\"",
            )
            .write("app/models/badge.rb", "class Badge < ApplicationRecord\nend\n")
            .write("app/models/visit.rb", "class Visit < ApplicationRecord\nend\n")
    };
    let insert = |model: &str| {
        let start = model.find("def _adapter_insert").expect("_adapter_insert");
        let end = model[start..].find("\n  end\n").map(|at| start + at).expect("end of _adapter_insert");
        model[start..end].to_string()
    };
    for target in [BuildTarget::Spinel, BuildTarget::Ruby] {
        let (emitted, errors) = app(POSTGRESQL_DATABASE_YML).emit(target);
        assert!(errors.is_empty(), "{target:?}: {errors:#?}");
        let badge = insert(&read(&emitted, "app/models/badge.rb"));
        assert!(badge.contains("RETURNING 0, label") && !badge.contains("rowid"), "{target:?}:\n{badge}");
        let visit = insert(&read(&emitted, "app/models/visit.rb"));
        assert!(visit.contains("RETURNING 0, recorded_at") && !visit.contains("rowid"), "{target:?}:\n{visit}");
    }
    let (emitted, errors) = app("test:\n  adapter: sqlite3\n  database: \":memory:\"\n").emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    assert!(insert(&read(&emitted, "app/models/badge.rb")).contains("RETURNING CAST(rowid AS TEXT), label"));
    assert!(insert(&read(&emitted, "app/models/visit.rb")).contains("RETURNING CAST(rowid AS TEXT), recorded_at"));
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
