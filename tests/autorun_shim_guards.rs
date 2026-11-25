//! The autorun shim's per-test guard — what it covers, and what it
//! catches.
//!
//! Both holes here were found by wiring Mocha into campfire's suite,
//! and both cost a whole FILE rather than a test:
//!
//!   * `__t.teardown` ran OUTSIDE the guard, one line below the comment
//!     explaining that anything outside it "would abort the file and
//!     hide every test behind it — the exact failure mode this shim
//!     exists to end". `mocha_verify` raises there on an unmet
//!     `expects`, and one of those took `membership_test` from 8 of 9
//!     to 0 of 9.
//!
//!   * the guard rescued StandardError, and `Mocha::ExpectationError <
//!     Exception` — deliberately, so a test's own `rescue => e` cannot
//!     swallow an unmet expectation. The shim was not swallowing it, it
//!     was MISSING it. Minitest's own runner rescues Exception here for
//!     the same reason.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use roundhouse::emit::ruby;
use roundhouse::ingest::ingest_app_from_tree;
use roundhouse::project::BuildTarget;

fn shim() -> String {
    let files: HashMap<PathBuf, Vec<u8>> = [
        (
            PathBuf::from("db/schema.rb"),
            b"ActiveRecord::Schema.define do\n  create_table \"posts\", force: :cascade do |t|\n    t.string \"body\", null: false\n  end\nend\n".to_vec(),
        ),
        (
            PathBuf::from("app/models/post.rb"),
            b"class Post < ApplicationRecord\nend\n".to_vec(),
        ),
        (
            PathBuf::from("test/models/post_test.rb"),
            b"require \"test_helper\"\n\nclass PostTest < ActiveSupport::TestCase\n  test \"one\" do\n    assert true\n  end\nend\n".to_vec(),
        ),
    ]
    .into_iter()
    .collect();
    let mut app = ingest_app_from_tree(files).expect("ingest");
    roundhouse::session::analyze_and_lower(&mut app);
    // `emit_spinel` is the entry that carries the autorun shim — the
    // ruby and spinel trees share the emitted test files.
    let files = ruby::emit_spinel(&app);
    files
        .iter()
        .find(|f| f.path.ends_with("post_test.rb"))
        .unwrap_or_else(|| {
            let paths: Vec<_> = files.iter().map(|f| f.path.clone()).collect();
            panic!("no post_test.rb in {paths:?}")
        })
        .content
        .clone()
}

#[test]
fn an_app_suite_asks_use_transactional_tests() {
    let src = shim();
    assert!(
        src.contains("if self.class.use_transactional_tests"),
        "an app suite loads TestBase and must honor the flag:\n{src}",
    );
    assert!(src.contains("__t.__txn_test_one"), "{src}");
}

#[test]
fn a_framework_file_does_not_call_use_transactional_tests() {
    let files: HashMap<PathBuf, Vec<u8>> = [
        (
            PathBuf::from("db/schema.rb"),
            b"ActiveRecord::Schema.define do\nend\n".to_vec(),
        ),
        (
            PathBuf::from("test/models/cookies_test.rb"),
            b"require \"test_helper\"\n\nclass CookiesTest < Minitest::Test\n  def test_one\n    assert true\n  end\nend\n".to_vec(),
        ),
    ]
    .into_iter()
    .collect();
    let mut app = ingest_app_from_tree(files).expect("ingest");
    roundhouse::session::analyze_and_lower(&mut app);
    let files = ruby::emit_spinel(&app);
    let src = files
        .iter()
        .find(|f| f.path.ends_with("cookies_test.rb"))
        .map(|f| f.content.as_str())
        .unwrap_or_else(|| panic!("no cookies test in {:?}", files.iter().map(|f| &f.path).collect::<Vec<_>>()));
    assert!(
        !src.contains("use_transactional_tests"),
        "a framework file never loads the helper that defines the method:\n{src}",
    );
    assert!(src.contains("__t.test_one"), "{src}");
}

#[test]
fn teardown_runs_inside_a_guard_of_its_own() {
    let src = shim();
    let after_rescue = src
        .split("__t.teardown")
        .next()
        .expect("no teardown in the shim");
    // The teardown call is preceded by its own `begin`, not left bare
    // at the top level of the file.
    assert!(
        after_rescue.trim_end().ends_with("begin"),
        "teardown must be guarded:\n{src}",
    );
}

/// It still has to RUN after a failing test — a stub installed on a
/// real object must not outlive the test that made it — so it lives in
/// a second guard rather than moving above the first rescue.
#[test]
fn teardown_is_not_folded_into_the_test_guard() {
    let src = shim();
    let test_guard = src.split("rescue Exception => __e").next().unwrap_or("");
    assert!(
        !test_guard.contains("__t.teardown"),
        "teardown must not sit inside the test's own guard:\n{src}",
    );
}

#[test]
fn the_guard_catches_exception_not_just_standard_error() {
    let src = shim();
    assert!(src.contains("rescue Exception => __e"), "{src}");
    assert!(
        !src.contains("rescue => __e"),
        "a bare rescue misses Mocha::ExpectationError:\n{src}",
    );
}

/// A test that already failed must not be counted twice when its
/// teardown fails too — `__failed` is the tally's numerator.
#[test]
fn a_failing_teardown_after_a_failing_test_counts_once() {
    let src = shim();
    assert!(src.contains("__ok = true"), "{src}");
    assert!(src.contains("__ok = false"), "{src}");
    assert!(src.contains("if __ok"), "{src}");
}

/// The whole emitted tree for one target, so the assertion covers what
/// the ruby and spinel lanes actually ship rather than the shared
/// `emit_spinel` entry (which is the spinel text).
fn target_test_file(target: BuildTarget) -> String {
    let files: HashMap<PathBuf, Vec<u8>> = [
        (
            PathBuf::from("db/schema.rb"),
            b"ActiveRecord::Schema.define do\n  create_table \"posts\", force: :cascade do |t|\n    t.string \"body\", null: false\n  end\nend\n".to_vec(),
        ),
        (
            PathBuf::from("app/models/post.rb"),
            b"class Post < ApplicationRecord\nend\n".to_vec(),
        ),
        (
            PathBuf::from("test/models/post_test.rb"),
            b"require \"test_helper\"\n\nclass PostTest < ActiveSupport::TestCase\n  test \"one\" do\n    assert_equal 1, Post.count\n  end\nend\n".to_vec(),
        ),
    ]
    .into_iter()
    .collect();
    let mut app = ingest_app_from_tree(files).expect("ingest");
    roundhouse::session::analyze_and_lower(&mut app);
    let files =
        roundhouse::project::target_files(&app, Path::new("."), target).expect("target files");
    // The two trees lay the same test out at different paths
    // (`test/models/post_test.rb` on ruby, `test/post_test.rb` on
    // spinel, which flattens the suite), so match on the stem.
    let paths: Vec<String> = files
        .iter()
        .map(|(p, _)| p.clone())
        .filter(|p| p.contains("post_test"))
        .collect();
    files
        .into_iter()
        .find(|(path, _)| path.ends_with("post_test.rb"))
        .map(|(_, content)| content)
        .unwrap_or_else(|| panic!("no post_test.rb emitted for {}: {paths:?}", target.as_str()))
}

/// A `FAIL` line carries the message only, so a suite ranking its walls
/// cannot tell where a failure happened. The ruby and jruby trees name
/// the exception class and the first backtrace frame on a SECOND line
/// (the `FAIL` line's own shape is pinned by parsers that split it on
/// its first `: `), with the emit root stripped so it reads relative.
///
/// Spinel AOT compiles that same shim text and has no
/// `Exception#backtrace`, so its tree must not grow the line.
#[test]
fn only_the_ruby_tree_names_the_exception_and_where_it_was_raised() {
    for target in [BuildTarget::Ruby, BuildTarget::Jruby] {
        let src = target_test_file(target);
        assert!(
            src.contains("puts \"  at \" + __e.class.name.to_s + \" \" + __frame"),
            "{} must print the exception class and frame:\n{src}",
            target.as_str(),
        );
        assert!(
            src.contains("__backtrace = __e.backtrace"),
            "{} must read the backtrace:\n{src}",
            target.as_str(),
        );
        assert!(
            src.contains("__frame = __frame.delete_prefix(Dir.pwd + \"/\")"),
            "{} must strip the emit root:\n{src}",
            target.as_str(),
        );
    }
    let spinel = target_test_file(BuildTarget::Spinel);
    assert!(
        !spinel.contains("backtrace") && !spinel.contains("  at "),
        "spinel AOT has no Exception#backtrace:\n{spinel}",
    );
}
