//! #1820: a fixture file under a subdirectory (`test/fixtures/admin/notes.yml`
//! for `Admin::Note`) was loaded in the Go test emit through the model's Ruby
//! constant (`r := NewAdmin::Note()`, `*Admin::Note`, `Admin::NoteFind(id)`), so
//! `fixtures_test.go` didn't parse and none of the app's Go tests built.

use super::emit_and_run;

/// real-blog with two namespaced models and their fixtures:
/// - `Admin::Note` (`admin_notes`: a required and a nullable string column,
///   and an integer with a default the fixture leaves out), loaded from
///   `test/fixtures/admin/notes.yml`, with a `::` inside a string value;
/// - `Admin::Tag` (`admin_tags`), loaded from the flat `test/fixtures/tags.yml`
///   through `_fixture: model_class:`, so the set's name (`tags`) doesn't give
///   the model's;
///
/// and a model test that reads both back through the count and the named
/// accessors.
pub fn app() -> emit_and_run::Overlay {
    let schema = std::fs::read_to_string("fixtures/real-blog/db/schema.rb").unwrap();
    let at = schema.find("  create_table").unwrap();
    let tables = "  create_table \"admin_notes\", force: :cascade do |t|\n    t.string \"name\", null: false\n    t.string \"body\"\n    t.integer \"priority\", default: 3, null: false\n  end\n\n  create_table \"admin_tags\", force: :cascade do |t|\n    t.string \"label\", null: false\n  end\n\n";
    emit_and_run::real_blog()
        .write("db/schema.rb", &format!("{}{tables}{}", &schema[..at], &schema[at..]))
        .write(
            "app/models/admin/note.rb",
            "module Admin\n  class Note < ApplicationRecord\n    self.table_name = \"admin_notes\"\n  end\nend\n",
        )
        .write(
            "app/models/admin/tag.rb",
            "module Admin\n  class Tag < ApplicationRecord\n    self.table_name = \"admin_tags\"\n  end\nend\n",
        )
        .write("test/fixtures/admin/notes.yml", "one:\n  name: \"A\"\n  body: \"B::C\"\n")
        .write("test/fixtures/tags.yml", "_fixture:\n  model_class: Admin::Tag\n\none:\n  label: \"T\"\n")
        .write("test/models/admin/note_test.rb", TEST)
}

const TEST: &str = "require \"test_helper\"\n\nclass Admin::NoteTest < ActiveSupport::TestCase\n  test \"the namespaced fixture loads\" do\n    assert_equal 1, Admin::Note.count\n    assert_equal \"A\", admin_notes(:one).name\n    assert_equal \"B::C\", admin_notes(:one).body\n    assert_equal 3, admin_notes(:one).priority\n    assert_equal 1, Admin::Tag.count\n    assert_equal \"T\", tags(:one).label\n  end\nend\n";
