//! `<=>` answers nil when a Float side is NaN, on either side and from an
//! Integer receiver too; equal numbers answer 0. On real-blog's Article,
//! run under tsx. The script names nil, NaN and undefined apart, since
//! JSON writes all three as `null`.

use super::emit_and_run::{self, Overlay};

const MODEL: &str = r#"
  #: (Float, Float) -> Integer?
  def self.float_order(a, b)
    a <=> b
  end

  #: (Integer, Integer) -> Integer
  def self.int_order(a, b)
    a <=> b
  end

  #: (Integer, Float) -> Integer?
  def self.mixed_order(a, b)
    a <=> b
  end
"#;

/// What the calls answer under CRuby, nil written as "nil", as JSON.
#[allow(dead_code)]
pub const SUMMARY: &str = r#"["nil","nil",-1,0,0,1,"nil"]"#;

/// The same calls, as the tsx script makes them, with nil, NaN and undefined named apart.
#[allow(dead_code)]
pub const CALLS: &str = "[Article.float_order(NaN, 1.0), Article.float_order(1.0, NaN), Article.float_order(1.5, 2.5), \
Article.float_order(2.5, 2.5), Article.int_order(2, 2), Article.int_order(3, 1), Article.mixed_order(1, NaN)]\
.map((x: unknown) => x === null ? \"nil\" : x === undefined ? \"undefined\" : Number.isNaN(x) ? \"NaN\" : x)";

pub fn overlay() -> Overlay {
    emit_and_run::real_blog().edit("app/models/article.rb", "class Article < ApplicationRecord\n", &format!("class Article < ApplicationRecord\n{MODEL}"))
}
