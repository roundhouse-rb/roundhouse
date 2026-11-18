//! `db:prepare` / `db:migrate` on the emitted binary, as `bin/rails` runs
//! them: a database the app built at one migration is brought forward by a
//! later build, keeping its rows. real-blog without its schema.rb, so its
//! own migrations (create_articles, create_comments with
//! `t.references :article, foreign_key: true`) are the schema.

use super::{emit_and_run, native_http};
use roundhouse::project::BuildTarget;
use std::path::Path;
use std::process::{Command, Output};

fn blog(tree: &Path, db: &Path, args: &[&str]) -> Output {
    Command::new(tree.join("build/bin/blog"))
        .args(args)
        .env("BLOG_DB", db)
        .current_dir(tree)
        .output()
        .expect("run the blog binary")
}

fn sql(db: &Path, query: &str) -> String {
    let out = Command::new("sqlite3")
        .arg(db)
        .arg(query)
        .output()
        .expect("run sqlite3");
    assert!(
        out.status.success(),
        "sqlite3 {query}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
#[ignore = "requires Spinel; run in its CI lane"]
fn db_migrate_brings_an_earlier_database_forward_keeping_its_rows() {
    // v1: the fixture's own migrations
    let (v1, errors) = emit_and_run::real_blog()
        .remove("db/schema.rb")
        .emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "v1 errors:\n{}", errors.join("\n"));
    native_http::build(&v1);
    // in the v1 tree, which outlives the v2 build below
    let db = v1.join("migrate_test.sqlite3");
    let prepared = blog(&v1, &db, &["db:prepare"]);
    assert!(prepared.status.success(), "db:prepare: {}", text(&prepared));
    assert_eq!(sql(&db, "SELECT count(*) FROM schema_migrations"), "2");
    assert_eq!(
        sql(
            &db,
            "SELECT value FROM ar_internal_metadata WHERE key = 'environment'"
        ),
        "development"
    );
    sql(
        &db,
        "INSERT INTO articles (title, body, created_at, updated_at) VALUES ('Kept', 'A body that stays', '2026-01-01', '2026-01-01');\
         INSERT INTO comments (article_id, commenter, body, created_at, updated_at) VALUES (1, 'ann', 'hi', '2026-01-01', '2026-01-01')",
    );

    // v2: two more migrations; one changes a column SQLite cannot alter in place
    let (v2, errors) = emit_and_run::real_blog()
        .remove("db/schema.rb")
        .write(
            "db/migrate/20991231000001_add_subtitle_to_articles.rb",
            "class AddSubtitleToArticles < ActiveRecord::Migration[8.1]\n  def change\n    add_column :articles, :subtitle, :string, default: \"\", null: false\n  end\nend\n",
        )
        .write(
            "db/migrate/20991231000002_comment_defaults.rb",
            "class CommentDefaults < ActiveRecord::Migration[8.1]\n  def change\n    add_index :articles, :title\n    change_column_default :comments, :commenter, from: nil, to: \"anonymous\"\n  end\nend\n",
        )
        .emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "v2 errors:\n{}", errors.join("\n"));
    native_http::build(&v2);

    // the server does not migrate: the pending ones are named, nothing changes
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
        .to_string();
    let mut server = Command::new(v2.join("build/bin/blog"))
        .args(["-p", port.as_str()])
        .env("BLOG_DB", &db)
        .current_dir(&*v2)
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("start the server");
    // wait (bounded) for the boot report; an early exit ends the stream
    let (tx, rx) = std::sync::mpsc::channel();
    let stderr = server.stderr.take().expect("server stderr");
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut boot_err = String::new();
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(line) => {
                boot_err.push_str(&line);
                boot_err.push('\n');
                if line.contains("pending migration(s)") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = server.kill();
    let status = server.wait().expect("server exit");
    boot_err.extend(rx.try_iter().map(|line| line + "\n"));
    let boot_err = format!("{boot_err}(server: {status})");
    assert!(
        boot_err.contains("2 pending migration(s): 20991231000001, 20991231000002"),
        "{boot_err}"
    );
    assert_eq!(sql(&db, "SELECT count(*) FROM schema_migrations"), "2");

    let migrated = blog(&v2, &db, &["db:migrate"]);
    assert!(migrated.status.success(), "db:migrate: {}", text(&migrated));
    assert!(
        text(&migrated).contains("2 migration(s) applied"),
        "{}",
        text(&migrated)
    );
    assert_eq!(sql(&db, "SELECT count(*) FROM schema_migrations"), "4");
    assert_eq!(
        sql(&db, "SELECT title || '|' || subtitle FROM articles"),
        "Kept|"
    );
    assert_eq!(
        sql(&db, "SELECT commenter || '|' || body FROM comments"),
        "ann|hi"
    );
    assert!(
        sql(&db, "SELECT sql FROM sqlite_master WHERE name = 'comments'")
            .contains("DEFAULT 'anonymous'")
    );
    assert_eq!(
        sql(
            &db,
            "SELECT \"table\" FROM pragma_foreign_key_list('comments')"
        ),
        "articles"
    );
    assert!(
        sql(&db, "SELECT name FROM sqlite_master WHERE type = 'index'")
            .contains("index_comments_on_article_id")
    );

    // nothing pending: a second run applies nothing
    let again = blog(&v2, &db, &["db:migrate"]);
    assert!(
        text(&again).contains("0 migration(s) applied"),
        "{}",
        text(&again)
    );
}
