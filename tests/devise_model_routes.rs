//! `devise_for :users` draws the routes of the modules `User` declares, as
//! `Devise::Mapping#used_routes` does: no `:confirmable`, no confirmation
//! routes. A module whose routes are not modeled refuses rather than drops
//! them, and a module another gem adds keeps Devise's default four.

use std::collections::HashMap;
use std::path::PathBuf;

use roundhouse::ingest::ingest_app_from_tree;

fn ingest(modules: &str) -> Result<Vec<String>, roundhouse::ingest::IngestError> {
    let user = format!("class User < ApplicationRecord\n  devise {modules}\nend\n");
    let files: Vec<(&str, String)> = vec![
        ("db/schema.rb", "ActiveRecord::Schema.define do\n  create_table \"users\" do |t|\n    t.string \"email\"\n  end\nend\n".into()),
        ("app/models/application_record.rb", "class ApplicationRecord < ActiveRecord::Base\nend\n".into()),
        ("app/models/user.rb", user),
        ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n".into()),
        ("config/routes.rb", "Rails.application.routes.draw do\n  devise_for :users\nend\n".into()),
    ];
    let tree: HashMap<PathBuf, Vec<u8>> =
        files.into_iter().map(|(p, c)| (PathBuf::from(p), c.into_bytes())).collect();
    let app = ingest_app_from_tree(tree)?;
    Ok(roundhouse::lower::flatten_routes(&app).iter().map(|r| r.as_name.clone()).collect())
}

#[test]
fn the_models_modules_choose_the_routes() {
    let names = ingest(":database_authenticatable, :registerable, :recoverable, :rememberable, :validatable").expect("ingest");
    for name in ["new_user_session", "destroy_user_session", "new_user_registration", "edit_user_password"] {
        assert!(names.iter().any(|n| n == name), "{name} missing: {names:?}");
    }
    assert!(!names.iter().any(|n| n.contains("confirmation")), "{names:?}");

    let names = ingest(":database_authenticatable").expect("ingest");
    assert!(names.iter().any(|n| n == "new_user_session"), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("registration") || n.contains("password")), "{names:?}");
}

#[test]
fn an_unmodeled_modules_routes_refuse() {
    let err = ingest(":database_authenticatable, :lockable").expect_err("lockable's unlock routes are not modeled");
    assert!(format!("{err:?}").contains(":lockable"), "{err:?}");
}

#[test]
fn another_gems_module_keeps_the_default_routes() {
    let names = ingest(":two_factor_authenticatable, :registerable").expect("ingest");
    assert!(names.iter().any(|n| n == "new_user_session") && names.iter().any(|n| n == "user_confirmation"), "{names:?}");
}
