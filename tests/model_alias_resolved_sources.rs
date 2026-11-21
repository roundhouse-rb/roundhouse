//! `lower::model_to_library::resolve_model_aliases`: the sources Rails
//! actually resolves an `alias` against once synthesis has run —
//! columns/associations/attr_accessor are invisible to the model
//! ingest walk, so resolving had to move here (see
//! `ModelBodyItem::Alias`'s doc for the full per-source matrix,
//! grounded against real Rails 8.1 at class-body time).
//!
//! One CRuby run per newly-resolved source: the alias answers exactly
//! what the target answers, same as `emit_and_run`'s other suites.
//! Reduced to Widget/Gadget, not the Mastodon shapes that motivated
//! this (relay.rb, keypair.rb, follow_request.rb, collection_item.rb).

#[path = "support/emit_and_run.rs"]
mod emit_and_run;

fn widgets_app() -> emit_and_run::Overlay {
    emit_and_run::empty_app()
        .write(
            "app/controllers/application_controller.rb",
            "class ApplicationController < ActionController::Base\nend\n",
        )
        .write("config/routes.rb", "Rails.application.routes.draw do\nend\n")
        .write(
            "app/models/application_record.rb",
            "class ApplicationRecord < ActiveRecord::Base\n  primary_abstract_class\nend\n",
        )
        .write(
            "db/schema.rb",
            r#"ActiveRecord::Schema.define do
  create_table "widgets", force: :cascade do |t|
    t.string "name"
    t.integer "state", default: 0, null: false
  end
  create_table "gadgets", force: :cascade do |t|
    t.integer "widget_id"
  end
end
"#,
        )
}

#[test]
fn alias_of_a_belongs_to_reader_answers_the_same_as_the_reader() {
    let run = widgets_app()
        .write(
            "app/models/widget.rb",
            "class Widget < ApplicationRecord\nend\n",
        )
        .write(
            "app/models/gadget.rb",
            "class Gadget < ApplicationRecord\n  belongs_to :widget, optional: true\n  alias owner widget\nend\n",
        )
        .run_ruby(
            r#"
widget = Widget.create!(name: "sprocket")
gadget = Gadget.create!(widget: widget)
raise "belongs_to alias" unless gadget.owner == gadget.widget
raise "belongs_to alias identity" unless gadget.owner.id == widget.id
"#,
        );
    run.assert_passes();
}

#[test]
fn alias_of_an_attr_accessor_answers_the_same_as_the_accessor() {
    let run = widgets_app()
        .write(
            "app/models/widget.rb",
            "class Widget < ApplicationRecord\n  attr_accessor :scratch\n  alias scratch2 scratch\nend\n",
        )
        .write("app/models/gadget.rb", "class Gadget < ApplicationRecord\nend\n")
        .run_ruby(
            r#"
widget = Widget.new
widget.scratch = 42
raise "attr_accessor alias" unless widget.scratch2 == 42
"#,
        );
    run.assert_passes();
}

#[test]
fn alias_of_an_enum_predicate_answers_the_same_as_the_predicate() {
    let run = widgets_app()
        .write(
            "app/models/widget.rb",
            "class Widget < ApplicationRecord\n  enum :state, { idle: 0, accepted: 1 }\n  alias enabled? accepted?\nend\n",
        )
        .write("app/models/gadget.rb", "class Gadget < ApplicationRecord\nend\n")
        .run_ruby(
            r#"
widget = Widget.create!(name: "w", state: :accepted)
raise "enum alias" unless widget.enabled? == widget.accepted?
raise "enum alias true" unless widget.enabled?
idle = Widget.create!(name: "i", state: :idle)
raise "enum alias false" if idle.enabled?
"#,
        );
    run.assert_passes();
}

#[test]
fn alias_of_an_inherited_active_record_method_forwards_to_it() {
    let run = widgets_app()
        .write(
            "app/models/widget.rb",
            "class Widget < ApplicationRecord\n  alias remove! destroy!\nend\n",
        )
        .write("app/models/gadget.rb", "class Gadget < ApplicationRecord\nend\n")
        .run_ruby(
            r#"
widget = Widget.create!(name: "w")
id = widget.id
widget.remove!
raise "inherited alias forwarder" unless Widget.where(id: id).count.zero?
"#,
        );
    run.assert_passes();
}

#[test]
fn alias_of_a_hand_written_method_still_works_through_the_new_path() {
    // Pre-existing behavior (the model walk always resolved a
    // hand-written-to-hand-written alias) — regression guard for the
    // move from ingest-time to lowering-time resolution.
    let run = widgets_app()
        .write(
            "app/models/widget.rb",
            "class Widget < ApplicationRecord\n  def greeting\n    \"hi \" + name.to_s\n  end\n  alias greet greeting\nend\n",
        )
        .write("app/models/gadget.rb", "class Gadget < ApplicationRecord\nend\n")
        .run_ruby(
            r#"
widget = Widget.new(name: "sprocket")
raise "hand-written alias" unless widget.greet == widget.greeting
"#,
        );
    run.assert_passes();
}

#[test]
fn alias_of_a_has_many_reader_answers_to_another_class() {
    // Called from another model's own method, so the analysis must
    // dispatch `parts` on `Widget`, not only the emitted model define it.
    let run = widgets_app()
        .write(
            "app/models/widget.rb",
            "class Widget < ApplicationRecord\n  has_many :gadgets, dependent: :destroy, inverse_of: :widget\n  alias parts gadgets\nend\n",
        )
        .write(
            "app/models/gadget.rb",
            "class Gadget < ApplicationRecord\n  belongs_to :widget\n\n  def sibling_count\n    widget.parts.size\n  end\nend\n",
        )
        .run_ruby(
            r#"
widget = Widget.create!(name: "sprocket")
first = Gadget.create!(widget: widget)
Gadget.create!(widget: widget)
raise "has_many alias" unless widget.parts.map(&:id).sort == widget.gadgets.map(&:id).sort
raise "has_many alias from another class" unless first.sibling_count == 2
"#,
        );
    run.assert_passes();
}

#[test]
fn an_alias_of_a_method_a_parent_redefines_forwards_to_the_parents() {
    let run = widgets_app()
        .write(
            "app/models/application_record.rb",
            "class ApplicationRecord < ActiveRecord::Base\n  primary_abstract_class\n\n  def reload\n    super\n  end\nend\n",
        )
        .write("app/models/widget.rb", "class Widget < ApplicationRecord\n  alias refresh reload\nend\n")
        .write(
            "app/models/gadget.rb",
            "class Gadget < ApplicationRecord\n  belongs_to :widget, optional: true\n\n  def widget_name\n    widget.refresh.name\n  end\nend\n",
        )
        .run_ruby(
            r#"
widget = Widget.create!(name: "before")
gadget = Gadget.create!(widget: widget)
Widget.where(id: widget.id).update_all(name: "after")
raise "refresh reloads" unless widget.refresh.name == "after"
raise "refresh from another class" unless gadget.widget_name == "after"
"#,
        );
    run.assert_passes();
}

#[test]
fn an_alias_of_a_method_a_parent_overrides_answers_the_overrides_value() {
    let run = widgets_app()
        .write(
            "app/models/application_record.rb",
            "class ApplicationRecord < ActiveRecord::Base\n  primary_abstract_class\n\n  def reload\n    \"ready\"\n  end\nend\n",
        )
        .write("app/models/widget.rb", "class Widget < ApplicationRecord\n  alias refresh reload\nend\n")
        .write(
            "app/models/gadget.rb",
            "class Gadget < ApplicationRecord\n  belongs_to :widget, optional: true\n\n  def widget_status\n    widget.refresh.upcase\n  end\nend\n",
        )
        .run_ruby(
            r#"
widget = Widget.create!(name: "w")
gadget = Gadget.create!(widget: widget)
raise "refresh answers the override" unless widget.refresh == "ready"
raise "refresh from another class" unless gadget.widget_status == "READY"
"#,
        );
    run.assert_passes();
}
