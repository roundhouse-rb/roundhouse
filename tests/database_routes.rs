//! Which role-and-shard routing the Spinel target carries, and what stays
//! refused. The emitted program's behavior is in
//! `tests/spinel_toolchain/database_routes.rs`; here are the admission
//! rules and the diagnostics.
//!
//! Carried: one abstract class under ActiveRecord::Base that declares a
//! literal `connects_to shards:` or `database:`, every model under it,
//! each configuration a literal SQLite file, and each writable one
//! naming the app's own schema file. Everything else keeps its error, on
//! every target; the carried shape keeps one on every target but Spinel.

#[path = "support/emit_and_run.rs"]
mod emit_and_run;
#[path = "support/database_routes.rs"]
mod contract;
#[path = "support/prerequisite.rs"]
mod prerequisite;

use roundhouse::project::BuildTarget;

fn shards() -> emit_and_run::Overlay {
    contract::app(contract::SHARDS, "class RouteReport\nend\n")
}

/// The Spinel tree's `runtime/db.rb` and the errors of its emit.
fn spinel_db(overlay: emit_and_run::Overlay) -> (String, Vec<String>) {
    let (emitted, errors) = overlay.emit(BuildTarget::Spinel);
    (std::fs::read_to_string(emitted.join("runtime/db.rb")).unwrap_or_else(|error| panic!("read emitted runtime: {error}; diagnostics: {errors:#?}")), errors)
}

fn generated(db: &str) -> &str {
    let from = db.find("# >>> generated: database-routes\n").expect("routes region");
    let to = db.find("# <<< generated: database-routes\n").expect("routes region end");
    &db[from..to]
}

#[test]
fn spinel_registers_each_shard_and_role_and_no_other_target_takes_the_app() {
    let (db, errors) = spinel_db(shards());
    assert!(errors.is_empty(), "{errors:#?}");
    let routes = generated(&db);
    for line in [
        "when \"development\", \"\"\n",
        "when \"test\"\n",
        "route(:left, :writing, ::AppDatabase.path_for_name(\"development\", \"left\"), false, true, n)\n",
        "route(:left, :reading, ::AppDatabase.path_for_name(\"development\", \"left_reader\"), true, false, n)\n",
        "route(:right, :writing, ::AppDatabase.path_for_name(\"development\", \"right\"), false, true, n)\n",
        "route(:right, :reading, ::AppDatabase.path_for_name(\"development\", \"right_reader\"), true, false, n)\n",
        "@default_shard = :left\n",
    ] {
        assert!(routes.contains(line), "missing {line:?} in:\n{routes}");
    }
    // An environment the file does not describe raises at boot.
    assert!(routes.contains("else\n      raise \"Db.configure: config/database.yml names no SQLite files to route `ApplicationRecord`"), "{routes}");

    let checked = shards().check_errors();
    assert_eq!(checked.len(), 1, "{checked:#?}");
    assert!(checked[0].contains("only the spinel target's runtime opens them"), "{checked:#?}");
    let (_, ruby) = shards().emit(BuildTarget::Ruby);
    assert!(ruby.iter().any(|e| e.contains("only the spinel target's runtime opens them")), "{ruby:#?}");
}

/// The emitted app's generated route region, run in Ruby inside `module Db`
/// as `Db.configure` runs it, with boot.rb's `AppDatabase` and an app's own
/// `Db::AppDatabase` beside it: one line per route, `shard role file`,
/// after the primary `Main.default_db_path` asks for.
fn routed_files(emitted: &std::path::Path, env: &str) -> String {
    let db = std::fs::read_to_string(emitted.join("runtime/db.rb")).unwrap();
    let boot = std::fs::read_to_string(emitted.join("boot.rb")).unwrap();
    let routes = generated(&db);
    let start = boot.find("module AppDatabase\n").expect("module AppDatabase");
    let resolver = &boot[start..start + boot[start..].find("\nend\n").expect("module end") + 5];
    let script = format!(
        "module Rails\n  def self.env_name\n    ENV[\"RAILS_ENV\"]\n  end\nend\n{resolver}\
         module Db\n  module AppDatabase\n    def self.path_for_name(env, name)\n      \"storage/decoy.sqlite3\"\n    end\n  end\n\n\
         \x20 def self.route(shard, role, path, replica, schema, n)\n    puts \"#{{shard}} #{{role}} #{{path}}\"\n  end\n\n  def self.run\n    n = 8\n{routes}\n  end\nend\n\
         puts AppDatabase.path_for(Rails.env_name.to_s.empty? ? \"development\" : Rails.env_name)\nDb.run\n"
    );
    let output = std::process::Command::new("ruby").env("RAILS_ENV", env).args(["-e", &script]).output().expect("run ruby");
    assert!(output.status.success(), "{script}\n{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The registrations read their files from boot.rb's resolver, beside the
/// one `Main.default_db_path` asks for the primary, by its root name: an
/// app's own `Db::AppDatabase` does not answer them. Run in Ruby, the
/// generated region routes each shard and role of `RAILS_ENV=production`
/// to the file config/database.yml names for it there.
#[test]
fn each_route_reads_its_file_from_the_resolver() {
    let development = contract::database_yml();
    let production = development.split("test:\n").next().unwrap().replacen("development:", "production:", 1).replace("storage/", "/data/app/");
    let (emitted, errors) = shards().write("config/database.yml", &format!("{development}{production}")).emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    let db = std::fs::read_to_string(emitted.join("runtime/db.rb")).unwrap();
    assert!(!generated(&db).contains("sqlite3\""), "a route names a file of its own:\n{}", generated(&db));
    assert_eq!(
        routed_files(&emitted, "production"),
        "/data/app/primary.sqlite3\nleft writing /data/app/left.sqlite3\nright writing /data/app/right.sqlite3\nleft reading /data/app/left.sqlite3\nright reading /data/app/right.sqlite3\n"
    );
    assert_eq!(
        routed_files(&emitted, ""),
        "storage/primary.sqlite3\nleft writing storage/left.sqlite3\nright writing storage/right.sqlite3\nleft reading storage/left.sqlite3\nright reading storage/right.sqlite3\n"
    );
}

/// A quoted `""` key names a configuration, as in Rails: a reading role
/// that names it reads its file, not the primary's.
#[test]
fn an_empty_configuration_name_is_a_name() {
    let yml = "test:\n  primary: { adapter: sqlite3, database: storage/primary.sqlite3 }\n  \"\": { adapter: sqlite3, database: storage/reader.sqlite3, replica: true }\n";
    let record = "class ApplicationRecord < ActiveRecord::Base\n  self.abstract_class = true\n  connects_to database: { writing: :primary, reading: :\"\" }\nend\n";
    let report = "class RouteReport\n  def self.run\n    ApplicationRecord.connected_to(role: :reading) { Note.count }\n  end\nend\n";
    let (emitted, errors) = contract::app(record, report).write("config/database.yml", yml).emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(routed_files(&emitted, "test"), "storage/primary.sqlite3\ndefault writing \ndefault reading storage/reader.sqlite3\n");
}

/// `AppDatabase.path_for_name` answers as Rails 8.1.4's
/// `configs_for(env_name:, name:, include_hidden: true).database` for each
/// SQLite configuration (`load_defaults` plays no part), including a
/// replica and an empty name, and nil where Rails finds none. Rails runs
/// on this file alone: RAILS_ENV pinned, no `DATABASE_URL` or
/// `<NAME>_DATABASE_URL` from the caller.
#[test]
fn the_named_lookup_answers_as_rails_configs_for() {
    let yml = "test:\n  primary: { adapter: sqlite3, database: storage/primary.sqlite3 }\n  reader: { adapter: sqlite3, database: storage/reader.sqlite3, replica: true }\n  \"\": { adapter: sqlite3, database: storage/empty.sqlite3, replica: true }\n  cache: { adapter: sqlite3, database: storage/cache.sqlite3 }\n\
               production:\n  primary: { adapter: sqlite3, database: /data/app/primary.sqlite3 }\n  reader: { adapter: sqlite3, database: /data/app/reader.sqlite3, replica: true }\n";
    let record = "class ApplicationRecord < ActiveRecord::Base\n  self.abstract_class = true\n  connects_to database: { writing: :primary, reading: :reader }\nend\n";
    let report = "class RouteReport\n  def self.run\n    ApplicationRecord.connected_to(role: :reading) { Note.count }\n  end\nend\n";
    let (emitted, errors) = contract::app(record, report).write("config/database.yml", yml).emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    let calls = [("test", "primary"), ("test", "reader"), ("test", ""), ("test", "cache"), ("test", "queue"), ("production", "primary"), ("production", "reader"), ("production", "cache"), ("staging", "primary")];
    let oracle = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/rails_oracle.rb");
    let lookups: String = calls.iter().map(|(env, name)| format!("p configs.configs_for(env_name: {env:?}, name: {name:?}, include_hidden: true)&.database\n")).collect();
    let script = format!(
        "require {oracle:?}\nexit 77 unless RailsOracle.available?(\"activerecord\")\nRailsOracle.activate(\"activerecord\")\n\
         require \"active_record\"\nrequire \"active_record/database_configurations\"\nrequire \"yaml\"\n\
         configs = ActiveRecord::DatabaseConfigurations.new(YAML.safe_load({yml:?}))\n{lookups}"
    );
    let mut command = std::process::Command::new("ruby");
    command.env("RAILS_ENV", "test").env_remove("RACK_ENV").env_remove("DATABASE_URL");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().ends_with("_DATABASE_URL") {
            command.env_remove(key);
        }
    }
    let rails = command.args(["-e", &script]).output().expect("run ruby");
    if rails.status.code() == Some(77) {
        prerequisite::missing("Rails oracle with activerecord");
        return;
    }
    assert!(rails.status.success(), "{}", String::from_utf8_lossy(&rails.stderr));
    let boot = std::fs::read_to_string(emitted.join("boot.rb")).unwrap();
    let start = boot.find("module AppDatabase\n").expect("module AppDatabase");
    let resolver = &boot[start..start + boot[start..].find("\nend\n").expect("module end") + 5];
    let probes: String = calls.iter().map(|(env, name)| format!("p AppDatabase.path_for_name({env:?}, {name:?})\n")).collect();
    let emitted_answers = std::process::Command::new("ruby").args(["-e", &format!("{resolver}{probes}")]).output().expect("run ruby");
    assert!(emitted_answers.status.success(), "{resolver}\n{}", String::from_utf8_lossy(&emitted_answers.stderr));
    assert_eq!(String::from_utf8_lossy(&emitted_answers.stdout), String::from_utf8_lossy(&rails.stdout), "{resolver}");
}

/// A shard whose name starts with a digit is written quoted, as Ruby
/// reads it: `:2026` does not parse.
#[test]
fn a_shard_named_with_a_leading_digit_is_quoted() {
    let record = contract::SHARDS.replace("left: { writing", "\"2026\": { writing");
    let (db, errors) = spinel_db(contract::app(&record, "class RouteReport\nend\n"));
    assert!(errors.is_empty(), "{errors:#?}");
    let routes = generated(&db);
    assert!(routes.contains("route(:\"2026\", :writing, ::AppDatabase.path_for_name(\"development\", \"left\"), false, true, n)\n"), "{routes}");
    assert!(routes.contains("route(:right, :writing, ::AppDatabase.path_for_name(\"development\", \"right\"), false, true, n)\n"), "{routes}");
    assert!(routes.contains("@default_shard = :\"2026\"\n"), "{routes}");
}

#[test]
fn an_unreadable_schema_does_not_admit_routes_in_survey() {
    roundhouse::ingest::survey::activate();
    let (db, errors) = spinel_db(shards().remove("db/schema.rb").write("db/schema.rb/unreadable", ""));
    let gaps = roundhouse::ingest::survey::drain();
    assert!(gaps.iter().any(|gap| gap.to_string().contains("file not readable")), "{gaps:?}");
    assert!(errors.iter().any(|error| error.contains("shard routing is not modeled")), "{errors:?}");
    assert_eq!(generated(&db), "# >>> generated: database-routes\n    ");
}

/// A routed writer must name the Ruby schema actually loaded for the primary.
#[test]
fn a_named_ruby_schema_admits_routes_with_the_same_schema() {
    let yml = contract::database_yml().replace("database: storage/primary.sqlite3\n", "database: storage/primary.sqlite3\n    schema_dump: catalog_schema.rb\n")
        .replace("schema_dump: schema.rb", "schema_dump: catalog_schema.rb");
    let schema = "ActiveRecord::Schema[8.1].define(version: 1) do\n  create_table :notes do |t|\n    t.string :tag\n  end\nend\n";
    let (db, errors) = spinel_db(shards().remove("db/schema.rb").write("db/catalog_schema.rb", schema).write("config/database.yml", &yml));
    assert!(errors.is_empty(), "{errors:#?}");
    assert!(generated(&db).contains("@default_shard"), "{}", generated(&db));
}

#[test]
fn routes_refuse_a_schema_file_different_from_the_loaded_primary_schema() {
    let yml = contract::database_yml().replace("database: storage/primary.sqlite3\n", "database: storage/primary.sqlite3\n    schema_dump: catalog_schema.rb\n");
    let schema = "ActiveRecord::Schema[8.1].define(version: 1) do\n  create_table :notes do |t|\n    t.string :tag\n  end\nend\n";
    let (db, errors) = spinel_db(shards().write("db/catalog_schema.rb", schema).write("config/database.yml", &yml));
    assert!(errors.iter().any(|e| e.contains("shard routing is not modeled")), "{errors:#?}");
    assert!(!generated(&db).lines().any(|line| line.trim_start().starts_with("route(")), "{db}");
}

/// A model's `prepend Concern` in a routed app stays the concern
/// splice's statement: the flag that claims it is not the one that marks
/// a writing block ingest read as its body, so the routes do not put a
/// `connected_to(role: :writing)` back around it.

/// A model's `prepend Concern` in a routed app stays the concern
/// splice's statement: the flag that claims it is not the one that marks
/// a writing block ingest read as its body, so the routes do not put a
/// `connected_to(role: :writing)` back around it.
#[test]
fn a_prepended_concern_in_a_routed_app_stays_a_prepend() {
    let overlay = shards()
        .write(
            "app/models/concerns/trimmed.rb",
            "module Trimmed\n  extend ActiveSupport::Concern\n\n  prepended do\n    before_save :trim_tag\n  end\n\n  private\n\n  def trim_tag\n    self.tag = tag.strip\n  end\nend\n",
        )
        .write("app/models/note.rb", "class Note < ApplicationRecord\n  prepend Trimmed\nend\n");
    let (emitted, errors) = overlay.emit(BuildTarget::Spinel);
    let note = std::fs::read_to_string(emitted.join("app/models/note.rb")).unwrap_or_default();
    assert!(errors.is_empty(), "{errors:#?}");
    assert!(note.contains("prepend Trimmed") && !note.contains("connected_to"), "{note}");
}

#[test]
fn an_app_on_one_database_keeps_an_empty_routes_region() {
    let one = contract::app("class ApplicationRecord < ActiveRecord::Base\n  self.abstract_class = true\nend\n", "class RouteReport\nend\n");
    let (db, errors) = spinel_db(one);
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(generated(&db), "# >>> generated: database-routes\n    ");
    // Naming only the primary routes nothing either: it is the database every app opens.
    let primary = contract::ROLES.replace("{ writing: :left, reading: :left_reader }", "{ writing: :primary }");
    let (db, errors) = spinel_db(contract::app(&primary, "class RouteReport\nend\n"));
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(generated(&db), "# >>> generated: database-routes\n    ");
}

/// A primary writer with a reading role checked clean before, on every
/// target, when nothing switched to the reader. It still does, with no
/// route; a reading block is what asks for the reader's pool.
#[test]
fn a_primary_writer_with_a_reading_role_routes_once_a_block_reads_from_it() {
    let record = contract::ROLES.replace("writing: :left, reading: :left_reader", "writing: :primary, reading: :left_reader");
    let (db, errors) = spinel_db(contract::app(&record, "class RouteReport\nend\n"));
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(generated(&db), "# >>> generated: database-routes\n    ");
    assert!(contract::app(&record, "class RouteReport\nend\n").check_errors().is_empty());

    let report = "class RouteReport\n  def self.run\n    ApplicationRecord.connected_to(role: :reading) { Note.count }\n  end\nend\n";
    let (db, errors) = spinel_db(contract::app(&record, report));
    assert!(errors.is_empty(), "{errors:#?}");
    let routes = generated(&db);
    assert!(routes.contains("route(:default, :writing, \"\", false, true, n)\n"), "{routes}");
    assert!(routes.contains("route(:default, :reading, ::AppDatabase.path_for_name(\"development\", \"left_reader\"), true, false, n)\n"), "{routes}");
    assert!(routes.contains("@default_shard = :default\n"), "{routes}");
    let (_, ruby) = contract::app(&record, report).emit(BuildTarget::Ruby);
    assert!(ruby.iter().any(|e| e.contains("only the spinel target's runtime opens them")), "{ruby:#?}");
}

/// Each shape outside the carried one keeps its error on Spinel, and no
/// route is registered for it.
#[test]
fn every_other_routing_shape_stays_refused_on_spinel() {
    let yml = contract::database_yml();
    let cases: Vec<(&str, emit_and_run::Overlay, &str)> = vec![
        (
            "a model outside the connection class",
            shards().write("app/models/audit.rb", "class Audit < ActiveRecord::Base\nend\n"),
            "shard routing is not modeled",
        ),
        (
            "a second connection class",
            shards().write("app/models/directory_record.rb", "class DirectoryRecord < ActiveRecord::Base\n  self.abstract_class = true\n  connects_to database: { writing: :right }\nend\n"),
            "shard routing is not modeled",
        ),
        (
            "a connection class that is not the primary class",
            contract::app(&contract::SHARDS.replace("ApplicationRecord", "TenantRecord"), "class RouteReport\nend\n")
                .write("app/models/note.rb", "class Note < TenantRecord\nend\n"),
            "shard routing is not modeled",
        ),
        (
            "a connection class beside another primary class",
            shards().write("app/models/main_record.rb", "class MainRecord < ApplicationRecord\n  primary_abstract_class\nend\n"),
            "shard routing is not modeled",
        ),
        (
            "a positional Hash, which Rails does not take",
            contract::app(&contract::SHARDS.replace("connects_to shards: {", "connects_to({ shards: {").replace("  }\nend", "  } })\nend"), "class RouteReport\nend\n"),
            "is not a literal",
        ),
        (
            "a file named by a url beside its database",
            shards().write("config/database.yml", &yml.replace("database: storage/left.sqlite3\n    schema_dump", "database: storage/left.sqlite3\n    url: sqlite3:storage/right.sqlite3\n    schema_dump")),
            "shard routing is not modeled",
        ),
        (
            "a role on a replica primary",
            contract::app(&contract::ROLES.replace("reading: :left_reader", "reading: :primary"), "class RouteReport\nend\n")
                .write("config/database.yml", &yml.replace("database: storage/primary.sqlite3\n", "database: storage/primary.sqlite3\n    replica: true\n")),
            "opens only the primary",
        ),
        (
            "a default shard set after the declaration",
            contract::app(&contract::SHARDS.replace("  }\nend", "  }\n  self.default_shard = :right\nend"), "class RouteReport\nend\n"),
            "changes the default a query runs on",
        ),
        (
            "a computed shard table",
            contract::app("class ApplicationRecord < ActiveRecord::Base\n  self.abstract_class = true\n  connects_to shards: SHARDS\nend\n", "class RouteReport\nend\n"),
            "shard routing is not modeled",
        ),
        (
            "a configured role name",
            contract::app(&contract::SHARDS.replace("reading: :left_reader", "archive: :left_reader"), "class RouteReport\nend\n"),
            "shard routing is not modeled",
        ),
        (
            "a writable database with a schema file of its own",
            shards().write("config/database.yml", &yml.replace("    database: storage/right.sqlite3\n    schema_dump: schema.rb\n", "    database: storage/right.sqlite3\n")),
            "shard routing is not modeled",
        ),
        (
            "a computed file",
            shards().write("config/database.yml", &yml.replace("database: storage/right.sqlite3\n    schema_dump", "database: <%= ENV[\"RIGHT\"] %>\n    schema_dump")),
            "shard routing is not modeled",
        ),
        (
            "an in-memory database",
            shards().write("config/database.yml", &yml.replace("database: storage/right.sqlite3\n    schema_dump", "database: \":memory:\"\n    schema_dump")),
            "shard routing is not modeled",
        ),
        (
            "a primary writer outside the default shard",
            contract::app(&contract::SHARDS.replace("right: { writing: :right, reading: :right_reader }", "right: { writing: :primary }"), "class RouteReport\nend\n"),
            "shard routing is not modeled",
        ),
        (
            "another adapter",
            shards().write("config/database.yml", &yml.replace("adapter: sqlite3\n    database: storage/right.sqlite3\n    schema_dump", "adapter: postgresql\n    database: storage/right.sqlite3\n    schema_dump")),
            "shard routing is not modeled",
        ),
    ];
    let mut missed = Vec::new();
    for (name, overlay, phrase) in cases {
        let (db, errors) = spinel_db(overlay);
        // A refused statement in an otherwise carried app leaves its routes; the error stops the build.
        if !errors.iter().any(|e| e.contains(phrase)) || (db.lines().any(|line| line.trim_start().starts_with("route(")) && !phrase.starts_with("changes the default")) {
            missed.push(format!("{name}: {errors:?}"));
        }
    }
    assert!(missed.is_empty(), "{}", missed.join("\n"));
}

/// In a carried app, a call the runtime would not run as Rails does is an
/// error at the call, each naming what it is.
#[test]
fn calls_outside_the_carried_scope_are_refused_by_name() {
    let cases: &[(&str, &str)] = &[
        ("ApplicationRecord.connected_to(role: \"reading\") { 1 }", "`connected_to` here is not a scope the routed runtime carries"),
        ("ApplicationRecord.connected_to(role: :archive) { 1 }", "`connected_to` here is not a scope the routed runtime carries"),
        ("Note.connected_to(shard: :left) { 1 }", "`connected_to` here is not a scope the routed runtime carries"),
        ("connected_to(shard: :left) { 1 }", "`connected_to` here is not a scope the routed runtime carries"),
        ("ApplicationRecord.connected_to(**options) { 1 }", "`connected_to` here is not a scope the routed runtime carries"),
        ("ApplicationRecord.connected_to(shard: :left, prevent_writes: flag) { 1 }", "`connected_to` here is not a scope the routed runtime carries"),
        ("ApplicationRecord.connected_to(shard: \"left\") { 1 }", "`connected_to` here is not a scope the routed runtime carries"),
        ("ApplicationRecord.connected_to(shard: :left, &block)", "`connected_to` here is not a scope the routed runtime carries"),
        ("ApplicationRecord.connected_to(shard: :left) { |x| x }", "`connected_to` here is not a scope the routed runtime carries"),
        ("ApplicationRecord.while_preventing_writes(flag) { 1 }", "`while_preventing_writes` here is not a scope the routed runtime carries"),
        ("ActiveRecord::Base.connected_to_many(ApplicationRecord, role: :reading) { 1 }", "role and shard switching is not modeled"),
        ("ApplicationRecord.connecting_to(role: :reading)", "role and shard switching is not modeled"),
        ("ApplicationRecord.prohibit_shard_swapping { 1 }", "role and shard switching is not modeled"),
        ("ActiveRecord::Base.connection_handler", "connection handler"),
        ("ApplicationRecord.connects_to(database: { writing: :left })", "registers a connection while the app runs"),
        ("ApplicationRecord.establish_connection(:right)", "registers a connection while the app runs"),
        ("ApplicationRecord.default_shard = :right", "changes the default a query runs on"),
    ];
    let mut missed = Vec::new();
    for (call, phrase) in cases {
        let report = format!("class RouteReport\n  def self.run(options, flag, &block)\n    {call}\n  end\nend\n");
        let (_, errors) = contract::app(contract::SHARDS, &report).emit(BuildTarget::Spinel);
        if !errors.iter().any(|e| e.contains(phrase)) {
            missed.push(format!("{call}: {errors:?}"));
        }
    }
    assert!(missed.is_empty(), "{}", missed.join("\n"));
}

/// A connection is routed when it is taken, so no receiver is refused
/// at build: not an app class's own `connection`, by name or through a
/// local, and not a model's.
#[test]
fn a_connection_method_is_not_refused_by_its_receiver() {
    let report = "class RouteReport\n  def self.run\n    source = CustomSource\n    owner = ApplicationRecord\n    ApplicationRecord.connected_to(shard: :right) { CustomSource.connection + source.connection + owner.connection.select_value(\"SELECT 1\").to_s }\n  end\nend\n";
    let app = contract::app(contract::SHARDS, report).write("app/models/custom_source.rb", "class CustomSource\n  def self.connection\n    \"app-value\"\n  end\nend\n");
    let (_, errors) = app.emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
}

/// In a routed app a switch in a view, a seed or a test helper is refused
/// by name: a view's block does not write its output, and a writing block
/// read as its body would stay one there. A reading block only a view has
/// still makes the app a routed one.
#[test]
fn a_switch_in_a_view_a_seed_or_a_test_helper_is_refused_by_name() {
    const PHRASE: &str = "in a view, a seed or a test helper is not carried";
    let view = |record: &str, erb: &str| {
        contract::app(record, "class RouteReport\nend\n")
            .write("config/routes.rb", "Rails.application.routes.draw do\n  get \"/notes\", to: \"notes#index\"\nend\n")
            .write("app/controllers/notes_controller.rb", "class NotesController < ApplicationController\n  def index\n  end\nend\n")
            .write("app/views/notes/index.html.erb", erb)
    };
    let primary_writer = contract::ROLES.replace("writing: :left, reading: :left_reader", "writing: :primary, reading: :left_reader");
    let cases = [
        ("a reading block in a view", view(contract::SHARDS, "<% ApplicationRecord.connected_to(role: :reading, shard: :right) do %><%= Note.count %><% end %>\n")),
        ("a writing block in a view", view(contract::SHARDS, "<%= ActiveRecord::Base.connected_to(role: :writing) { Note.count } %>\n")),
        ("a view's the only reading block, on a primary writer", view(&primary_writer, "<%= ApplicationRecord.connected_to(role: :reading) { Note.count } %>\n")),
        ("a seed", shards().write("db/seeds.rb", "ApplicationRecord.connected_to(shard: :right) { Note.create!(tag: \"seed\") }\n")),
        (
            "a test helper",
            shards().write("test/models/note_test.rb", "require \"test_helper\"\n\nclass NoteTest < ActiveSupport::TestCase\n  def on_right\n    ApplicationRecord.while_preventing_writes { Note.count }\n  end\n\n  test \"counts\" do\n    assert_equal 0, on_right\n  end\nend\n"),
        ),
    ];
    let mut missed = Vec::new();
    for (name, overlay) in cases {
        let (_, errors) = overlay.emit(BuildTarget::Spinel);
        if !errors.iter().any(|e| e.contains(PHRASE)) {
            missed.push(format!("{name}: {errors:?}"));
        }
    }
    assert!(missed.is_empty(), "{}", missed.join("\n"));
    // Without a switch in them, a routed app's view and seed are untouched.
    let plain = view(contract::SHARDS, "<%= Note.count %>\n").write("db/seeds.rb", "Note.create!(tag: \"seed\")\n");
    let report = "class RouteReport\n  def self.run\n    ApplicationRecord.connected_to(shard: :right) { Note.count }\n  end\nend\n";
    let (_, errors) = plain.write("app/models/route_report.rb", report).emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
}

/// A parameter's default runs on a call, so a switch there is read like
/// one in the body.
#[test]
fn a_switch_in_a_parameter_default_is_read() {
    let report = "class RouteReport\n  def self.run(count = ApplicationRecord.connected_to(role: \"reading\") { Note.count })\n    count\n  end\nend\n";
    let (_, errors) = contract::app(contract::SHARDS, report).emit(BuildTarget::Spinel);
    assert!(errors.iter().any(|e| e.contains("`connected_to` here is not a scope the routed runtime carries")), "{errors:#?}");
}

/// The default isolation level written out, with a comment after it, is
/// the default.
#[test]
fn the_default_isolation_level_with_a_trailing_comment_is_not_refused() {
    let application = "require_relative \"boot\"\nrequire \"rails/all\"\n\nmodule Routed\n  class Application < Rails::Application\n    config.load_defaults 8.1\n    config.active_support.isolation_level = :thread # the default\n  end\nend\n";
    let (_, errors) = shards().write("config/application.rb", application).emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    let (_, errors) = shards().write("config/application.rb", &application.replace(":thread # the default", ":fiber # not the default")).emit(BuildTarget::Spinel);
    assert!(errors.iter().any(|e| e.contains("isolation_level")), "{errors:#?}");
}

/// A handle's temporary lease trims its statement cache before the
/// connection goes back, as a request's lease does.
#[test]
fn a_handles_lease_trims_before_release() {
    let src = include_str!("../runtime/spinel/db.rb");
    let leave = &src[src.find("def self.leave_slot").expect("leave_slot")..];
    let trim = leave.find("conn.trim!").expect("leave_slot trims");
    let release = leave.find("release(frame.lease)").expect("leave_slot releases");
    assert!(trim < release);
    // Each under an `ensure` of its own, so a cleanup that raises loses neither the lease nor the scope outside.
    let pop = leave.find("stack.pop").expect("leave_slot pops its frame");
    assert!(leave[trim..release].contains("ensure") && leave[release..pop].contains("ensure"));
}

/// A block whose parameters make ingest rebuild its body (a trailing
/// parameter after a splat, a destructured one, a block-local) keeps a
/// writing block that is its only statement as one statement, so a routed
/// app puts the call back: with one statement in it, and with two.
#[test]
fn a_writing_block_in_a_rebuilt_block_body_is_restored() {
    let blocks = [
        ("a post parameter, one statement", "|*head, tail|", "Note.create!(tag: tail.to_s)"),
        ("a post parameter", "|*head, tail|", "Note.create!(tag: tail.to_s); Note.count"),
        ("a destructured parameter", "|(first, second)|", "Note.create!(tag: first.to_s); Note.count"),
        ("a block-local", "|row; tag|", "Note.create!(tag: tag.to_s + row.to_s); Note.count"),
    ];
    let mut missed = Vec::new();
    for (name, params, body) in blocks {
        let report = format!("class RouteReport\n  def self.run(rows)\n    ApplicationRecord.connected_to(role: :reading) do\n      rows.each do {params}\n        ActiveRecord::Base.connected_to(role: :writing) {{ {body} }}\n      end\n    end\n  end\nend\n");
        let (emitted, errors) = contract::app(contract::SHARDS, &report).emit(BuildTarget::Spinel);
        let text = std::fs::read_to_string(emitted.join("app/models/route_report.rb")).unwrap_or_default();
        if !errors.is_empty() || !text.contains("connected_to(role: :writing)") {
            missed.push(format!("{name}: {errors:?}\n{text}"));
        }
    }
    assert!(missed.is_empty(), "{}", missed.join("\n"));
}

/// The settings scan reads assignments: a mention in a string or a
/// comparison is not a configured role name.
#[test]
fn a_mention_of_a_routing_setting_is_not_a_setting() {
    let application = |line: &str| format!("require_relative \"boot\"\nrequire \"rails/all\"\n\nmodule Routed\n  class Application < Rails::Application\n    config.load_defaults 8.1\n    {line}\n  end\nend\n");
    for line in ["message = \"config.active_record.reading_role = :archive\"", "config.x.banner = \"reading_role\"", "config.x.primary = ActiveRecord.reading_role == :reading", "config.x.database_resolver = \"mine\""] {
        let (_, errors) = shards().write("config/application.rb", &application(line)).emit(BuildTarget::Spinel);
        assert!(errors.is_empty(), "{line}: {errors:#?}");
    }
    for line in ["config.active_record().reading_role = :archive", "config.active_record.reading_role = :replica", "config.active_record.reading_role = :archive; config.active_support.isolation_level = :thread"] {
        let (_, errors) = shards().write("config/application.rb", &application(line)).emit(BuildTarget::Spinel);
        assert!(errors.iter().any(|e| e.contains("database_routes_setting") && e.contains("reading_role")), "{errors:#?}");
    }
}

/// A controller action's parameter default is read like its body.
#[test]
fn a_switch_in_an_actions_parameter_default_is_read() {
    let app = shards()
        .write("config/routes.rb", "Rails.application.routes.draw do\n  get \"/notes\", to: \"notes#index\"\nend\n")
        .write("app/controllers/notes_controller.rb", "class NotesController < ApplicationController\n  def index(count = ApplicationRecord.connected_to(role: \"reading\") { Note.count })\n    head :ok\n  end\nend\n");
    let (_, errors) = app.emit(BuildTarget::Spinel);
    assert!(errors.iter().any(|e| e.contains("`connected_to` here is not a scope the routed runtime carries")), "{errors:#?}");
}

/// Options that cannot be read are refused by name even when they are the
/// app's only switch: the routes are kept for it, not dropped with it.
#[test]
fn an_unreadable_switch_alone_keeps_the_app_routed_and_is_refused() {
    let record = contract::ROLES.replace("writing: :left, reading: :left_reader", "writing: :primary, reading: :left_reader");
    for call in ["ApplicationRecord.connected_to(options) { Note.count }", "ActiveRecord::Base.connected_to(**options) { Note.count }"] {
        let report = format!("class RouteReport\n  def self.run(options)\n    {call}\n  end\nend\n");
        let (_, errors) = contract::app(&record, &report).emit(BuildTarget::Spinel);
        assert!(errors.iter().any(|e| e.contains("`connected_to` here is not a scope the routed runtime carries")), "{call}: {errors:#?}");
    }
}

/// The shared Db contract declares what the connection handle calls.
#[test]
fn the_db_contract_declares_the_handles_slot_calls() {
    let rbs = include_str!("../runtime/ruby/db.rbs");
    for method in ["connection_slot", "enter_slot", "leave_slot"] {
        assert!(rbs.contains(&format!("def self.{method}:")), "runtime/ruby/db.rbs does not declare {method}");
    }
}

/// The spellings of a carried scope check clean on Spinel: a rooted
/// constant, either class, the keywords in any order.
#[test]
fn the_carried_scope_checks_clean_in_each_spelling() {
    let report = "class RouteReport\n  def self.run(shard)\n    ::ActiveRecord::Base.connected_to(shard: shard, role: :reading) { Note.count }\n    ::ApplicationRecord.connected_to(prevent_writes: true, role: :writing) { Note.count }\n    ApplicationRecord.while_preventing_writes(true) { Note.count }\n    ActiveRecord::Base.while_preventing_writes { Note.count }\n    ActiveRecord::Base.connected_to(role: :writing) { Note.count }\n    Note.transaction { Note.count }\n    Note.current_shard\n  end\nend\n";
    let (_, errors) = contract::app(contract::SHARDS, report).emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn a_writing_scope_in_a_scope_parameter_default_is_restored() {
    let note = "class Note < ApplicationRecord\n  scope :chosen, ->(row = ActiveRecord::Base.connected_to(role: :writing) { Note.create!(tag: 'default').id }) { where(id: row) }\nend\n";
    let (emitted, errors) = shards().write("app/models/note.rb", note).emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    let text = std::fs::read_to_string(emitted.join("app/models/note.rb")).expect("emitted note");
    assert!(text.contains("ActiveRecord::Base.connected_to(role: :writing)"), "{text}");
}

fn model_expression_roots() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("has_many scope", "has_many :children, -> { where(tag: BODY) }, class_name: 'Note', foreign_key: :parent_id", "'writing'"),
        ("has_one scope", "has_one :child, -> { where(tag: BODY) }, class_name: 'Note', foreign_key: :parent_id", "'writing'"),
        ("belongs_to scope", "belongs_to :parent, -> { where(tag: BODY) }, class_name: 'Note', optional: true", "'writing'"),
        ("belongs_to default", "belongs_to :parent, class_name: 'Note', optional: true, default: -> { BODY }", "Note.first"),
        ("extension body", "has_many :children, class_name: 'Note', foreign_key: :parent_id do\n    def probe\n      BODY\n    end\n  end", "1"),
        ("extension default", "has_many :children, class_name: 'Note', foreign_key: :parent_id do\n    def probe(value = BODY)\n      value\n    end\n  end", "1"),
        ("validation if", "validates :tag, presence: true, if: -> { BODY }", "true"),
        ("validation unless", "validates :tag, presence: true, unless: -> { BODY }", "false"),
        ("inclusion callable", "validates :tag, inclusion: { in: -> { BODY } }", "['writing']"),
        ("exclusion callable", "validates :tag, exclusion: { in: -> { BODY } }", "['forbidden']"),
        ("comparison callable", "validates :parent_id, comparison: { greater_than: -> { BODY } }", "0"),
        ("callback condition", "before_validation :probe, if: -> { BODY }\n  def probe\n    true\n  end", "true"),
        ("callback block", "before_validation do\n    BODY\n  end", "true"),
        ("attribute default", "attribute :label, :string, default: -> { BODY }", "ApplicationRecord.current_role.to_s"),
        ("class scope", "scope :chosen, -> { where(tag: BODY) }", "'writing'"),
        ("class attribute default", "cattr_accessor :setting, default: -> { BODY }", "1"),
    ]
}

fn routed_expression_model(declaration: &str) -> emit_and_run::Overlay {
    let schema = "ActiveRecord::Schema[8.1].define(version: 1) do\n  create_table :notes do |t|\n    t.string :tag\n    t.integer :parent_id\n  end\nend\n";
    shards().write("db/schema.rb", schema).write("app/models/note.rb", &format!("class Note < ApplicationRecord\n  {declaration}\nend\n"))
}

fn emitted_model_sources(dir: &std::path::Path) -> String {
    let mut source = String::new();
    for entry in std::fs::read_dir(dir).expect("emitted model directory") {
        let path = entry.unwrap().path();
        if path.is_dir() { source.push_str(&emitted_model_sources(&path)); }
        else if path.extension().is_some_and(|ext| ext == "rb") { source.push_str(&std::fs::read_to_string(path).unwrap()); }
    }
    source
}

#[test]
fn writing_scopes_are_restored_in_every_model_expression_root() {
    let mut missed = Vec::new();
    for (name, template, value) in model_expression_roots() {
        let body = format!("ActiveRecord::Base.connected_to(role: :writing) {{ {value} }}");
        let (emitted, errors) = routed_expression_model(&template.replace("BODY", &body)).emit(BuildTarget::Spinel);
        let source = emitted_model_sources(&emitted.join("app/models"));
        if !errors.is_empty() || !source.contains("ActiveRecord::Base.connected_to(role: :writing)") { missed.push(format!("{name}: {errors:?}")); }
    }
    assert!(missed.is_empty(), "{}", missed.join("\n"));
}

#[test]
fn switches_in_every_model_expression_root_are_refused_by_name() {
    let mut missed = Vec::new();
    for (name, template, value) in model_expression_roots() {
        let body = format!("Note.connected_to(shard: :right) {{ {value} }}");
        let (_, errors) = routed_expression_model(&template.replace("BODY", &body)).emit(BuildTarget::Spinel);
        if !errors.iter().any(|error| error.contains("`connected_to` here is not a scope the routed runtime carries")) { missed.push(format!("{name}: {errors:?}")); }
    }
    assert!(missed.is_empty(), "{}", missed.join("\n"));
}
