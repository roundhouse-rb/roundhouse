//! A model's `alias new old` answers to the rest of the app, not only to
//! the emitted model: a controller's `post.notes` over
//! `alias notes comments` dispatches to the association. Every alias
//! `lower::model_to_library::resolve_model_aliases` resolves takes the
//! type of what it copies, and one it refuses stays unknown: typing it
//! would claim a method the emitted model does not have.

use std::collections::HashMap;
use std::path::PathBuf;

use roundhouse::analyze::{diagnose, DiagnosticKind};
use roundhouse::ingest::ingest_app_from_tree;

const SCHEMA: &str = r#"ActiveRecord::Schema.define do
  create_table "posts", force: :cascade do |t|
    t.string "title"
    t.integer "author_id"
  end
  create_table "comments", force: :cascade do |t|
    t.text "body"
    t.integer "post_id"
  end
  create_table "authors", force: :cascade do |t|
    t.string "name"
  end
end
"#;

const APPLICATION_RECORD: &str = "class ApplicationRecord < ActiveRecord::Base\n  primary_abstract_class\nend\n";

/// `(receiver, method)` for each failed dispatch, after analysis.
fn failures(post: &str, controller_body: &str) -> Vec<String> {
    failures_with(APPLICATION_RECORD, post, controller_body)
}

fn failures_with(application_record: &str, post: &str, controller_body: &str) -> Vec<String> {
    let controller = format!("class PostsController < ApplicationController\n{controller_body}\nend\n");
    let files: Vec<(&str, &str)> = vec![
        ("db/schema.rb", SCHEMA),
        ("app/models/application_record.rb", application_record),
        ("app/models/post.rb", post),
        ("app/models/comment.rb", "class Comment < ApplicationRecord\n  belongs_to :post\nend\n"),
        ("app/models/author.rb", "class Author < ApplicationRecord\nend\n"),
        ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
        ("app/controllers/posts_controller.rb", controller.as_str()),
        ("config/routes.rb", "Rails.application.routes.draw do\n  get \"/posts/:id\", to: \"posts#show\"\nend\n"),
    ];
    let tree: HashMap<PathBuf, Vec<u8>> = files.iter().map(|(p, c)| (PathBuf::from(p), c.as_bytes().to_vec())).collect();
    let mut app = ingest_app_from_tree(tree).expect("ingest");
    roundhouse::session::analyze_and_lower(&mut app);
    diagnose(&app)
        .into_iter()
        .filter_map(|d| match d.kind {
            DiagnosticKind::SendDispatchFailed { method, recv_ty } => {
                Some(format!("{}#{}", roundhouse::ide::render_ty(&recv_ty), method.as_str()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn an_alias_of_a_has_many_reader_answers_with_the_association() {
    let f = failures(
        "class Post < ApplicationRecord\n  has_many :comments, dependent: :destroy, inverse_of: :post\n  alias notes comments\nend\n",
        "  def show\n    post = Post.find(params[:id])\n    @bodies = post.notes.map(&:body)\n    @first = post.notes.first&.body\n  end",
    );
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn an_alias_of_a_belongs_to_reader_answers_with_the_record() {
    let f = failures(
        "class Post < ApplicationRecord\n  belongs_to :author, optional: true\n  alias writer author\nend\n",
        "  def show\n    @name = Post.find(params[:id]).writer&.name\n  end",
    );
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn an_alias_of_a_hand_written_method_answers_with_its_return() {
    let f = failures(
        "class Post < ApplicationRecord\n  def headline\n    title.to_s\n  end\n  alias heading headline\nend\n",
        "  def show\n    @heading = Post.find(params[:id]).heading.upcase\n  end",
    );
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn an_alias_of_an_attr_accessor_answers() {
    let f = failures(
        "class Post < ApplicationRecord\n  attr_accessor :draft\n  alias scratch draft\nend\n",
        "  def show\n    @scratch = Post.find(params[:id]).scratch\n  end",
    );
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn an_alias_of_an_alias_answers() {
    let f = failures(
        "class Post < ApplicationRecord\n  has_many :comments\n  alias notes comments\n  alias remarks notes\nend\n",
        "  def show\n    @first = Post.find(params[:id]).remarks.first&.body\n  end",
    );
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn an_alias_of_a_method_a_parent_redefines_answers_with_the_parents() {
    let f = failures_with(
        "class ApplicationRecord < ActiveRecord::Base\n  primary_abstract_class\n\n  def reload\n    super\n  end\nend\n",
        "class Post < ApplicationRecord\n  alias refresh reload\nend\n",
        "  def show\n    @title = Post.find(params[:id]).refresh.title\n  end",
    );
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn an_alias_of_a_method_a_parent_overrides_answers_with_the_overrides_return() {
    // The emitted `refresh` forwards to `reload`, which reaches the
    // override: a String, not the record.
    let application_record = "class ApplicationRecord < ActiveRecord::Base\n  primary_abstract_class\n\n  def reload\n    \"ready\"\n  end\nend\n";
    let post = "class Post < ApplicationRecord\n  alias refresh reload\nend\n";
    let string = failures_with(application_record, post, "  def show\n    @status = Post.find(params[:id]).refresh.upcase\n  end");
    assert!(string.is_empty(), "{string:?}");
    let record = failures_with(application_record, post, "  def show\n    @title = Post.find(params[:id]).refresh.title\n  end");
    assert_eq!(record, vec!["String#title"]);
}

#[test]
fn an_alias_that_replaces_an_association_reader_leaves_the_reader() {
    // The lowering refuses `alias comments author` (it would break the
    // methods the association generates), so `comments` stays the
    // collection. `saved_comments` copied the association before it, and
    // `remarks` copies `saved_comments`. Each call names a method no
    // receiver has, so the failures show the types themselves.
    let f = failures(
        "class Post < ApplicationRecord\n  has_many :comments\n  belongs_to :author, optional: true\n  alias saved_comments comments\n  alias comments author\n  alias remarks saved_comments\nend\n",
        "  def show\n    post = Post.find(params[:id])\n    @remarks = post.remarks.first&.missing_on_comment\n    @saved = post.saved_comments.first&.missing_on_comment\n    @comments = post.comments.first&.missing_on_comment\n  end",
    );
    assert_eq!(f, vec!["Comment#missing_on_comment", "Comment#missing_on_comment", "Comment#missing_on_comment"]);
}

#[test]
fn an_alias_the_lowering_refuses_stays_unknown() {
    // A column reader: Rails raises NameError at the alias line (`alias_attribute` is the form that works).
    let column = failures(
        "class Post < ApplicationRecord\n  alias heading title\nend\n",
        "  def show\n    @heading = Post.find(params[:id]).heading\n  end",
    );
    assert_eq!(column, vec!["Post#heading"]);
    // A target declared only after the alias line.
    let later = failures(
        "class Post < ApplicationRecord\n  alias notes comments\n  has_many :comments\nend\n",
        "  def show\n    @notes = Post.find(params[:id]).notes\n  end",
    );
    assert_eq!(later, vec!["Post#notes"]);
}
