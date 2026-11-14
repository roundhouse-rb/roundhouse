//! The Spinel `Db` shim over spinel-pg (runtime/spinel/db_pg.rb).
//!
//! The gate compiles the Db contract suite (tests/db_contract/) next to
//! the shim with Spinel, seeded with the shim's RBS, and runs it against
//! a live PostgreSQL: the contract cases every shim passes, the cases
//! any PostgreSQL Db passes, and db_pg.rb's own (its lane file,
//! tests/db_contract/db_pg_lane.rb). The suite drives the `Db` surface
//! directly; it and its lanes are described in tests/support/db_contract.rs.
//!
//! An emitted app whose primary database is PostgreSQL ships the shim as
//! its runtime/db.rb (`project::apply_postgresql_tree`, emission checked
//! in tests/postgresql_spinel_target.rs). The second gate compiles one of
//! that app's tests with Spinel and runs it on the server.
//!
//!   SPINEL_PG_DIR=/path/to/spinel-pg \
//!   ROUNDHOUSE_PG_TEST_URL=postgres://postgres:secret@127.0.0.1:5432/postgres \
//!   cargo test --test spinel_pg_db -- --ignored --nocapture
//!
//! Without ROUNDHOUSE_PG_TEST_URL the gate says so and skips, unless
//! ROUNDHOUSE_PG_TEST_REQUIRED is set, as CI sets it; a missing
//! SPINEL_PG_DIR fails it. Each run gets a schema of its own
//! (tests/support/pg_test_url.rs), dropped when it finishes.

#[path = "support/db_contract.rs"]
mod db_contract;
#[path = "support/emit_and_run.rs"]
mod emit_and_run;

use db_contract::{Backend, Runtime, lanes, pg_test_url};
use roundhouse::project::BuildTarget;
use std::collections::BTreeSet;
use std::process::Command;

#[test]
#[ignore = "requires Spinel, SPINEL_PG_DIR (a spinel-pg checkout) and ROUNDHOUSE_PG_TEST_URL (PostgreSQL)"]
fn spinel_pg_shims_pass_the_contract_and_postgres_cases() {
    let mut ran = 0;
    for lane in lanes(Runtime::Spinel, Backend::Postgres) {
        db_contract::run_lane(lane);
        ran += 1;
    }
    assert!(ran > 0, "no Spinel PostgreSQL lane");
}

const POSTGRESQL_DATABASE_YML: &str = "default: &default\n  adapter: postgresql\n  encoding: unicode\n\ndevelopment:\n  <<: *default\n  database: blog_development\n\ntest:\n  <<: *default\n  database: blog_test\n";

/// The blog on PostgreSQL, with a boolean column a lowered `where`
/// compares with a literal, a table with no primary key, and three tests
/// of its own: that literal is PostgreSQL's `TRUE` (SQLite's `1` is an
/// error against a boolean), a row goes into the keyless table (which has
/// no sequence to read a key from), and the test runs in the schema its
/// helper made for this process.
fn postgresql_blog() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .write("config/database.yml", POSTGRESQL_DATABASE_YML)
        .edit(
            "db/schema.rb",
            "    t.text \"body\"\n    t.datetime \"created_at\", null: false\n    t.datetime \"updated_at\", null: false\n  end\n\n  create_table \"comments\"",
            "    t.text \"body\"\n    t.boolean \"published\", default: false, null: false\n    t.datetime \"created_at\", null: false\n    t.datetime \"updated_at\", null: false\n  end\n\n  create_table \"stars\", id: false, force: :cascade do |t|\n    t.bigint \"article_id\", null: false\n  end\n\n  create_table \"comments\"",
        )
        .write("app/models/star.rb", "class Star < ApplicationRecord\nend\n")
        .edit(
            "app/models/article.rb",
            "class Article < ApplicationRecord\n",
            "class Article < ApplicationRecord\n  def self.published_count\n    where(published: true).count\n  end\n\n",
        )
        .edit(
            "test/models/article_test.rb",
            "class ArticleTest < ActiveSupport::TestCase\n",
            "class ArticleTest < ActiveSupport::TestCase\n  test \"counts published articles\" do\n    assert_equal 0, Article.published_count\n    articles(:one).update!(published: true)\n    assert_equal 1, Article.published_count\n  end\n\n  test \"stars an article in a table with no key\" do\n    Star.create!(article_id: articles(:one).id)\n    assert_equal 1, Star.where(article_id: articles(:one).id).count\n  end\n\n  test \"runs in a schema of its own\" do\n    schema = Article.connection.select_all(\"SELECT current_schema() AS name\").to_a.first[\"name\"]\n    assert schema.start_with?(\"rh_test_\"), schema\n  end\n\n",
        )
}

/// The emitted blog's model test, compiled with Spinel against spinel-pg
/// and run on a server: its helper makes the process a schema of its own
/// (search_path through PGOPTIONS, which spinel-pg sends on each
/// connection) and loads the boot DDL and the fixtures there; the tests
/// read, insert, validate, update and destroy through the PostgreSQL SQL
/// the emit lowered; the schema is dropped at exit.
#[test]
#[ignore = "requires Spinel, SPINEL_PG_DIR (a spinel-pg checkout) and ROUNDHOUSE_PG_TEST_URL (PostgreSQL)"]
fn an_emitted_apps_test_runs_natively_on_postgresql() {
    let Some(db) = pg_test_url::pg_test_db("spinel_pg_emitted_app") else { return };
    if std::env::var_os("RH_SPINEL_BUNDLE_COLLECT").is_some() {
        panic!("spinel bundle collect: not bundled");
    }
    let include = match std::env::var("SPINEL_PG_DIR") {
        Ok(dir) if !dir.is_empty() => dir,
        _ => panic!("SPINEL_PG_DIR must name a spinel-pg checkout"),
    };
    let (emitted, errors) = postgresql_blog().emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    // As the emitted Makefile's spinel-test recipe compiles a test, with
    // spinel-pg found where `spin` would put its include path.
    let compiler = std::env::var("SPINEL").unwrap_or_else(|_| "spinel".into());
    let compiled = Command::new(&compiler)
        .args(["-I", &include, "-O", "0", "--no-inline-hot", "test/article_test.rb", "-o", "article_test"])
        .current_dir(&*emitted)
        .output()
        .expect("spawn spinel");
    assert!(
        compiled.status.success(),
        "spinel test/article_test.rb in {}:\n{}{}",
        emitted.display(),
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
    let stdout = db_contract::run_bounded(
        Command::new(emitted.join("article_test"))
            .env("DATABASE_URL", &db.url)
            .env_remove("PGOPTIONS")
            .env_remove("DATABASE_POOL_SIZE")
            .env_remove("RH_SQL_TRACE")
            .current_dir(&*emitted),
        db_contract::LANE_DEADLINE,
        "Spinel did not leave the compiled test behind",
    );
    assert!(stdout.contains("ArticleTest: 7 tests passed"), "{stdout}");
}

/// `(owner, name)` for every `def` in a Ruby or RBS source, with `self.`
/// kept on singleton methods. Owners are the top-level `class`/`module`
/// lines, which is all these two files use.
fn definitions(src: &str, rbs: bool) -> BTreeSet<(String, String)> {
    let mut owner = String::new();
    let mut out = BTreeSet::new();
    for line in src.lines() {
        for keyword in ["class ", "module "] {
            if let Some(rest) = line.strip_prefix(keyword) {
                owner = rest.split_whitespace().next().unwrap_or("").to_string();
            }
        }
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("def ") else {
            continue;
        };
        let end = rest
            .find(|c: char| {
                if rbs {
                    c == ':'
                } else {
                    c == '(' || c.is_whitespace()
                }
            })
            .unwrap_or(rest.len());
        out.insert((owner.clone(), rest[..end].trim().to_string()));
    }
    out
}

/// Every method the shim (db_pg.rb, pg_errors.rb, pg_driver.rb) defines
/// carries a signature: the contract in runtime/ruby/db.rbs, or the
/// shim's own db_pg.rbs / pg_errors.rbs / pg_driver.rbs. All of them
/// parse as RBS.
#[test]
fn db_pg_rbs_declares_every_method() {
    let read =
        |path: &str| std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let contract = read("runtime/ruby/db.rbs");
    let mut ruby = String::new();
    let mut own = String::new();
    for stem in [
        "runtime/spinel/db_pg",
        "runtime/spinel/pg_errors",
        "runtime/spinel/pg_driver",
    ] {
        ruby.push_str(&read(&format!("{stem}.rb")));
        ruby.push('\n');
        let rbs = read(&format!("{stem}.rbs"));
        roundhouse::rbs::parse_app_signatures(&rbs)
            .unwrap_or_else(|e| panic!("{stem}.rbs does not parse: {e}"));
        own.push_str(&rbs);
        own.push('\n');
    }
    roundhouse::rbs::parse_app_signatures(&contract)
        .unwrap_or_else(|e| panic!("runtime/ruby/db.rbs does not parse: {e}"));
    let mut declared = definitions(&own, true);
    declared.extend(definitions(&contract, true));
    let defined = definitions(&ruby, false);
    assert!(
        defined.len() > 60,
        "found only {} definitions",
        defined.len()
    );
    let missing: Vec<String> = defined
        .difference(&declared)
        .map(|(owner, name)| format!("{owner}#{name}"))
        .collect();
    assert!(
        missing.is_empty(),
        "the PostgreSQL shim defines methods with no RBS signature:\n  {}",
        missing.join("\n  ")
    );
    let stale: Vec<String> = definitions(&own, true)
        .difference(&defined)
        .map(|(owner, name)| format!("{owner}#{name}"))
        .collect();
    assert!(
        stale.is_empty(),
        "the PostgreSQL shim RBS declares methods the shim does not define:\n  {}",
        stale.join("\n  ")
    );
}
