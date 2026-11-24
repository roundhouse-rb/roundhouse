//! `<col>_previously_was` for a column the last save did not change: the
//! value the column held before that save, not nil. The Rails Guides
//! "Getting Started" store reads `inventory_count_previously_was.zero?`
//! in an `after_update_commit`, so an update of the name alone raised
//! NoMethodError on nil. A record with no save yet answers nil. The
//! expected values are ActiveRecord 8.1.4's answers.

use super::emit_and_run;

fn app() -> emit_and_run::Overlay {
    emit_and_run::empty_app()
        .write("app/models/application_record.rb", r##"class ApplicationRecord < ActiveRecord::Base
  self.abstract_class = true
end
"##)
        .write("app/models/product.rb", r##"class Product < ApplicationRecord
end
"##)
        .write("app/models/probe.rb", r##"module Probe
  def self.check(label, ok)
    raise "#{label} differs from Rails" unless ok
  end

  def self.run
    a = Product.create!(name: "a", stock: 3)
    a.update!(stock: 0)
    check("the changed column", a.stock_previously_was == 3)
    check("the unchanged column", a.name_previously_was == "a")
    a.update!(name: "b")
    check("unchanged, after another column changed", a.stock_previously_was == 0)
    check("changed, after another column changed", a.name_previously_was == "a")
    a.save!
    check("a save of no change", a.stock_previously_was == 0 && a.name_previously_was == "b")

    q = Product.find(a.id)
    check("hydrated, no save yet", q.stock_previously_was.nil? && q.name_previously_was.nil?)
    q.update!(name: "c")
    check("hydrated, the unchanged column", q.stock_previously_was == 0)
    check("hydrated, the changed column", q.name_previously_was == "b")

    # a direct write after the save moves no answer
    q.update_column(:stock, 5)
    check("after update_column of the unchanged column", q.stock_previously_was == 0)
    q.update_columns(name: "d")
    check("after update_columns of the changed column", q.name_previously_was == "b")

    # a rolled-back save is no last save
    r = Product.find(a.id)
    r.stock = 7
    Product.transaction do
      r.save!
      check("inside the transaction", r.name_previously_was == "d")
      raise ActiveRecord::Rollback
    end
    check("after a rollback of the save", r.name_previously_was.nil?)

    # an Array goes out as a copy: a caller that changes the answer inside
    # the transaction of the save does not change the baseline that a
    # rollback puts back, so a later change of the record is still pending
    l = Product.create!(name: "l", stock: 1, labels: ["a"])
    l = Product.find(l.id)
    Product.transaction do
      l.update!(stock: 2)
      l.labels_previously_was << "b"
      check("the answer is a copy", l.labels_previously_was == ["a"])
      raise ActiveRecord::Rollback
    end
    l.labels = ["a", "b"]
    check("pending after the rollback", l.attribute_changed?("labels"))

    n = Product.new(name: "n", stock: 1)
    check("new, no save yet", n.stock_previously_was.nil? && n.name_previously_was.nil?)
  end
end
"##)
        .write("db/schema.rb", r##"ActiveRecord::Schema.define do
  create_table "products", force: :cascade do |t|
    t.string "name"
    t.integer "stock"
    t.string "labels", array: true
  end
end
"##)
}

const SCRIPT: &str = "Probe.run\nputs \"probe passed\"\n";

#[test]
fn previously_was_answers_the_value_of_an_unchanged_column() {
    let run = app().run_ruby(SCRIPT);
    run.assert_passes();
    assert!(run.stdout.contains("probe passed"), "{}", run.stdout);
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn previously_was_answers_the_value_of_an_unchanged_column_on_spinel() {
    let run = app().run_spinel_with_db(SCRIPT);
    run.assert_passes();
    assert!(run.stdout.contains("probe passed"), "{}", run.stdout);
}
