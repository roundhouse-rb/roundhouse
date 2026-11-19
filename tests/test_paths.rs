#[path = "support/scratch.rs"]
mod scratch;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use roundhouse::app::App;
use roundhouse::ingest::{IngestError, ingest_app, ingest_app_from_tree};

fn temp_dir() -> PathBuf {
    std::env::temp_dir().canonicalize().expect("resolve temp dir")
}

fn unique_tmp_dir(name: &str) -> PathBuf {
    // macOS $TMPDIR sits under the /var -> /private/var symlink; ingest
    // reports resolved paths, so the root must be resolved too.
    scratch::dir(&format!("roundhouse_{name}"))
}

fn test_source(class_name: &str) -> String {
    format!(
        "class {class_name} < ActiveSupport::TestCase\n  test \"runs\" do\n    assert_equal 1, 1\n  end\nend\n"
    )
}

fn ruby_file(path: &str, class_name: &str) -> (PathBuf, String) {
    (PathBuf::from(path), test_source(class_name))
}

fn text_file(path: &str, text: &str) -> (PathBuf, String) {
    (PathBuf::from(path), text.to_string())
}

fn ingest_files(files: impl IntoIterator<Item = (PathBuf, String)>) -> Result<App, IngestError> {
    let tree: HashMap<PathBuf, Vec<u8>> = files
        .into_iter()
        .map(|(path, text)| (path, text.into_bytes()))
        .collect();
    ingest_app_from_tree(tree)
}

fn module_names(app: &App) -> Vec<String> {
    let mut names: Vec<_> = app
        .test_modules
        .iter()
        .map(|module| module.name.0.as_str().to_string())
        .collect();
    names.sort();
    names
}

fn assert_modules(app: &App, expected: &[&str]) {
    let actual = module_names(app);
    let mut expected: Vec<_> = expected.iter().map(|name| name.to_string()).collect();
    expected.sort();
    assert_eq!(actual, expected);
}

fn default_and_custom_tests() -> Vec<(PathBuf, String)> {
    vec![
        ruby_file("test/models/model_default_test.rb", "ModelDefaultTest"),
        ruby_file(
            "test/controllers/controller_default_test.rb",
            "ControllerDefaultTest",
        ),
        ruby_file("test/helpers/helper_default_test.rb", "HelperDefaultTest"),
        ruby_file(
            "test/channels/channel_default_test.rb",
            "ChannelDefaultTest",
        ),
        ruby_file("test/lib/lib_default_test.rb", "LibDefaultTest"),
        // Not `*_test.rb`: Rails' default glob does not run them, so only
        // a configured root reads them.
        ruby_file("test/unit/unit_custom.rb", "UnitCustomTest"),
        ruby_file("test/utils/utils_custom.rb", "UtilsCustomTest"),
    ]
}

const DEFAULT_MODULES: &[&str] = &[
    "ModelDefaultTest",
    "ControllerDefaultTest",
    "HelperDefaultTest",
    "ChannelDefaultTest",
    "LibDefaultTest",
];

const ALL_STANDARD_MODULES: &[&str] = &[
    "ModelDefaultTest",
    "ControllerDefaultTest",
    "HelperDefaultTest",
    "ChannelDefaultTest",
    "LibDefaultTest",
    "UnitCustomTest",
    "UtilsCustomTest",
    "QualitySpecsTest",
];

#[test]
fn rails_test_roots_include_test_lib_by_default() {
    let app = ingest_files(default_and_custom_tests()).expect("ingest tree");
    assert_modules(&app, DEFAULT_MODULES);
}

/// What `bin/rails test` runs with no arguments: `test/**/*_test.rb`,
/// less `test/{system,dummy,fixtures}`. Before this, a test under
/// `test/integration` or an app's own folder was dropped silently.
#[test]
fn rails_default_glob_reaches_every_test_folder() {
    let mut files = default_and_custom_tests();
    files.extend([
        ruby_file("test/integration/flow_test.rb", "FlowTest"),
        ruby_file("test/middleware/guard_test.rb", "GuardTest"),
        ruby_file("test/script/admin/reset_test.rb", "ResetTest"),
        ruby_file("test/top_level_test.rb", "TopLevelTest"),
        // Not run by Rails' default glob.
        ruby_file("test/integration/not_a_test_file.rb", "NotATestFileTest"),
        ruby_file("test/system/browser_test.rb", "BrowserTest"),
        ruby_file("test/dummy/test/dummy_test.rb", "DummyTest"),
        ruby_file("test/fixtures/files/fixture_test.rb", "FixtureTest"),
        // Read, but its base has no harness: reported, not emitted.
        text_file(
            "test/mailers/notice_mailer_test.rb",
            "class NoticeMailerTest < ActionMailer::TestCase\n  test \"sends\" do\n  end\nend\n",
        ),
    ]);
    let app = ingest_files(files).expect("ingest tree");
    let mut expected = DEFAULT_MODULES.to_vec();
    expected.extend(["FlowTest", "GuardTest", "ResetTest", "TopLevelTest"]);
    assert_modules(&app, &expected);
    let drops: Vec<&str> = app.test_drops.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(
        drops,
        ["`NoticeMailerTest < ActionMailer::TestCase`: the emitted tests have no harness for ActionMailer::TestCase, so its tests are not emitted"]
    );
}

/// Test classes on bases with no harness, reached every way the ingest
/// handles, beside ones that keep theirs.
fn unharnessed_tree() -> Vec<(PathBuf, String)> {
    vec![
        ruby_file("test/models/model_default_test.rb", "ModelDefaultTest"),
        text_file(
            "test/mailers/notice_test.rb",
            "class NoticeTest < BaseMailerTest\n  KINDS.each do |kind|\n    test \"#{kind}\" do\n    end\n  end\nend\n",
        ),
        text_file(
            "test/mailers/kinds_mailer_test.rb",
            "class KindsMailerTest < ActionMailer::TestCase\n  KINDS.each do |kind|\n    test \"#{kind}\" do\n    end\n  end\nend\n",
        ),
        // A reopen in a later file is the same class, still left out, its
        // body (a loop the ingest would refuse) unread.
        text_file(
            "test/mailers/kinds_more_test.rb",
            "class KindsMailerTest\n  MORE.each do |kind|\n    test \"#{kind}\" do\n    end\n  end\nend\n",
        ),
        text_file(
            "test/test_helper.rb",
            // `HelperChainTest` is on a base left out earlier in this file.
            "class BaseMailerTest < ActionMailer::TestCase\nend\nclass HelperChainTest < BaseMailerTest\n  KINDS.each do |kind|\n    test \"#{kind}\" do\n    end\n  end\nend\nclass HarnessedBase < ActiveSupport::TestCase\nend\nclass OuterBase < ActionMailer::TestCase\nend\nmodule Mailers\n  class Base < ActionMailer::TestCase\n  end\n  class HarnessedBase < ActionMailer::TestCase\n  end\n  class OuterBase < ActiveSupport::TestCase\n  end\nend\n",
        ),
        // `OuterBase` here is the kept `Mailers::OuterBase`, which shadows
        // the left-out top-level one.
        text_file(
            "test/mailers/mailers/shadowed_test.rb",
            "module Mailers\n  class ShadowedTest < OuterBase\n    test \"kept\" do\n    end\n  end\nend\n",
        ),
        // A chain within one test file, and a reopen there.
        text_file(
            "test/mailers/chain_test.rb",
            "class LocalMailerTest < ActionMailer::TestCase\nend\nclass ChainTest < LocalMailerTest\n  KINDS.each do |kind|\n    test \"#{kind}\" do\n    end\n  end\nend\nclass LocalMailerTest\n  MORE.each do |kind|\n    test \"#{kind}\" do\n    end\n  end\nend\n",
        ),
        // Compact `class Mailers::CompactTest` looks `HarnessedBase` up at
        // the top level only, where it has a harness.
        text_file(
            "test/mailers/mailers/compact_test.rb",
            "class Mailers::CompactTest < HarnessedBase\n  test \"kept\" do\n  end\nend\n",
        ),
        // `Base` is `Mailers::Base` only by Ruby's lookup in the module.
        text_file(
            "test/mailers/mailers/digest_test.rb",
            "module Mailers\n  class DigestTest < Base\n    KINDS.each do |kind|\n      test \"#{kind}\" do\n      end\n    end\n  end\nend\n",
        ),
        // `::HarnessedBase` is the top-level one, whatever the module holds.
        text_file(
            "test/mailers/mailers/rooted_test.rb",
            "module Mailers\n  class RootedTest < ::HarnessedBase\n    test \"kept\" do\n    end\n  end\nend\n",
        ),
    ]
}

/// A base with no harness is reported however its class is reached: a
/// base the helper defines on one, and a class on that base, are left
/// out too, and a body construct the ingest would refuse is not read.
#[test]
fn an_unharnessed_base_is_reported_through_a_helper_base() {
    let app = ingest_files(unharnessed_tree()).expect("ingest tree");
    assert_modules(
        &app,
        &[
            "ModelDefaultTest",
            "HarnessedBase",
            "Mailers::OuterBase",
            "Mailers::RootedTest",
            "Mailers::CompactTest",
            "Mailers::ShadowedTest",
        ],
    );
    let mut dropped: Vec<&str> = app.test_drops.iter().map(|d| d.message.split('`').nth(1).unwrap()).collect();
    dropped.sort();
    assert_eq!(
        dropped,
        [
            "BaseMailerTest < ActionMailer::TestCase",
            "ChainTest < LocalMailerTest",
            "HelperChainTest < BaseMailerTest",
            "KindsMailerTest < ActionMailer::TestCase",
            "LocalMailerTest < ActionMailer::TestCase",
            "Mailers::Base < ActionMailer::TestCase",
            "Mailers::DigestTest < Mailers::Base",
            "Mailers::HarnessedBase < ActionMailer::TestCase",
            "NoticeTest < BaseMailerTest",
            "OuterBase < ActionMailer::TestCase",
        ]
    );
}

/// Under `--continue` a left-out class's body is not read either, so a
/// construct in it adds no survey gap beside the warning.
#[test]
fn a_left_out_class_adds_no_survey_gap() {
    roundhouse::ingest::survey::activate();
    let app = ingest_files(unharnessed_tree());
    let gaps = roundhouse::ingest::survey::drain();
    assert_eq!(app.expect("ingest tree").test_drops.len(), 10);
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn configured_roots_add_app_relative_test_folders() {
    let mut files = default_and_custom_tests();
    files.push(ruby_file(
        "quality/specs/nested/probe.rb",
        "QualitySpecsTest",
    ));
    files.push(text_file(
        "roundhouse.yml",
        "test_paths:\n  - test/unit\n  - test/utils\n  - quality/specs\n",
    ));

    let app = ingest_files(files).expect("ingest tree");
    assert_modules(&app, ALL_STANDARD_MODULES);
}

#[test]
fn empty_configuration_keeps_defaults_and_adds_no_custom_roots() {
    for config in ["", "{}\n", "test_paths: []\n"] {
        let mut files = default_and_custom_tests();
        files.push(text_file("roundhouse.yml", config));
        let app = ingest_files(files).expect("ingest tree");
        assert_modules(&app, DEFAULT_MODULES);
    }
}

#[test]
fn overlapping_and_repeated_roots_do_not_duplicate_modules() {
    let nested_source = format!(
        "{}{}",
        test_source("NestedFirstTest"),
        test_source("NestedSecondTest")
    );
    let app = ingest_files([
        ruby_file("test/models/model_default_test.rb", "ModelDefaultTest"),
        (PathBuf::from("test/unit/nested/probe.rb"), nested_source),
        text_file(
            "roundhouse.yml",
            "test_paths:\n  - test/unit\n  - test/unit\n  - ./test/unit/\n  - test/unit/nested\n  - test/models\n",
        ),
    ])
    .expect("ingest tree");

    assert_modules(
        &app,
        &["ModelDefaultTest", "NestedFirstTest", "NestedSecondTest"],
    );
}

#[test]
fn missing_directories_add_no_test_roots() {
    let mut files = default_and_custom_tests();
    files.push(ruby_file("test/not_a_directory.rb", "NotADirectoryTest"));
    files.push(text_file(
        "roundhouse.yml",
        "test_paths:\n  - test/unit/missing\n",
    ));

    let app = ingest_files(files).expect("ingest tree");
    assert_modules(&app, DEFAULT_MODULES);
}

#[test]
fn configured_file_path_is_a_configuration_error() {
    let error = ingest_files([
        ruby_file("test/unit.rb", "UnitTest"),
        text_file("roundhouse.yml", "test_paths: [test/unit.rb]\n"),
    ])
    .expect_err("a configured file must not be treated as a directory");

    let IngestError::Parse { file, message } = error else {
        panic!("expected a config parse error, got {error:?}");
    };
    assert_eq!(file, "roundhouse.yml");
    assert!(message.contains("directories"), "{message}");
}

#[cfg(unix)]
#[test]
fn configured_symlink_root_is_rejected() {
    use std::os::unix::fs::symlink;

    let root = unique_tmp_dir("test_paths_symlink_root");
    let app_root = root.join("app");
    let outside = root.join("outside");
    std::fs::create_dir_all(app_root.join("test")).expect("create app test directory");
    std::fs::create_dir_all(&outside).expect("create outside directory");
    std::fs::write(
        outside.join("outside_test.rb"),
        test_source("OutsideRootTest"),
    )
    .expect("write outside test");
    symlink(&outside, app_root.join("test/external")).expect("create external directory link");
    std::fs::write(
        app_root.join("roundhouse.yml"),
        "test_paths: [test/external]\n",
    )
    .expect("write config");

    let error = ingest_app(&app_root).expect_err("external symlink root must fail");
    let IngestError::Parse { file, message } = error else {
        panic!("expected a config parse error, got {error:?}");
    };
    assert_eq!(file, app_root.join("roundhouse.yml").display().to_string());
    assert!(message.contains("symbolic link"), "{message}");
    std::fs::remove_dir_all(root).expect("remove temp app");
}

#[cfg(unix)]
#[test]
fn recursive_discovery_skips_symlinks_outside_the_app() {
    use std::os::unix::fs::symlink;

    let root = unique_tmp_dir("test_paths_recursive_symlinks");
    let app_root = root.join("app");
    let test_root = app_root.join("quality/specs");
    let outside_dir = root.join("outside_dir");
    let outside_file = root.join("outside_file.rb");
    std::fs::create_dir_all(&test_root).expect("create test root");
    std::fs::create_dir_all(&outside_dir).expect("create outside directory");
    std::fs::write(test_root.join("local_test.rb"), test_source("LocalTest"))
        .expect("write local test");
    std::fs::write(
        outside_dir.join("outside_dir_test.rb"),
        test_source("OutsideDirTest"),
    )
    .expect("write outside directory test");
    std::fs::write(&outside_file, test_source("OutsideFileTest")).expect("write outside file test");
    symlink(&outside_dir, test_root.join("external_dir")).expect("create external directory link");
    symlink(&outside_file, test_root.join("external_file_test.rb"))
        .expect("create external file link");
    std::fs::write(
        app_root.join("roundhouse.yml"),
        "test_paths: [quality/specs]\n",
    )
    .expect("write config");

    let app = ingest_app(&app_root).expect("ingest app");
    assert_modules(&app, &["LocalTest"]);
    std::fs::remove_dir_all(root).expect("remove temp app");
}

#[test]
fn invalid_configuration_stops_ingestion_with_the_config_path() {
    let invalid_configs = [
        ("test_paths: [\n", None),
        ("test_paths: test/unit\n", None),
        ("unknown_key: true\n", None),
        ("test_paths: [\"\"]\n", Some("\"\"")),
        ("test_paths: [\".\"]\n", Some("\".\"")),
        ("test_paths: [\"../tests\"]\n", Some("\"../tests\"")),
        ("test_paths: [\"/tmp/tests\"]\n", Some("\"/tmp/tests\"")),
    ];

    for (config, rejected_path) in invalid_configs {
        let error = ingest_files([
            ruby_file("test/models/model_default_test.rb", "ModelDefaultTest"),
            text_file("roundhouse.yml", config),
        ])
        .expect_err("invalid configuration must fail ingestion");
        match error {
            IngestError::Parse { file, message } => {
                assert_eq!(file, "roundhouse.yml");
                if let Some(path) = rejected_path {
                    assert_eq!(
                        message,
                        format!(
                            "test_paths entries must be non-empty app-relative directories without '..': {path}"
                        )
                    );
                }
            }
            other => panic!("expected a config parse error, got {other:?}"),
        }
    }
}

fn test_source_paths(app: &App, root: Option<&Path>) -> Vec<String> {
    let mut paths: Vec<_> = app
        .sources
        .iter()
        .filter_map(|source| {
            let path = Path::new(&source.path);
            let relative = match root {
                Some(root) => path.strip_prefix(root).ok()?,
                None => path,
            };
            (relative
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("rb"))
            .then(|| relative.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    paths.sort();
    paths
}

#[test]
fn filesystem_and_map_vfs_select_the_same_configured_tests() {
    let root = unique_tmp_dir("test_paths_parity");
    let files = vec![
        ruby_file("test/models/default_test.rb", "DefaultTest"),
        ruby_file("quality/specs/nested/custom_test.rb", "CustomTest"),
        text_file(
            "roundhouse.yml",
            "test_paths: [quality/specs, ./quality/specs]\n",
        ),
    ];

    for (relative, content) in &files {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("file parent")).expect("create directory");
        std::fs::write(path, content).expect("write test app file");
    }

    let fs_app = ingest_app(&root).expect("ingest filesystem app");
    let map_app = ingest_files(files).expect("ingest map app");
    assert_eq!(module_names(&fs_app), module_names(&map_app));
    assert_eq!(
        test_source_paths(&fs_app, Some(&root)),
        test_source_paths(&map_app, None)
    );
    assert_eq!(
        test_source_paths(&map_app, None),
        vec![
            "quality/specs/nested/custom_test.rb".to_string(),
            "test/models/default_test.rb".to_string(),
        ]
    );

    std::fs::remove_dir_all(root).expect("remove temp app");
}
