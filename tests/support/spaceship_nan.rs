//! `<=>` answers nil when a Float side is NaN, on either side and from an
//! Integer receiver too; equal numbers answer 0. A side that is not a
//! number answers nil too (an untyped String or boolean, a nil), and two
//! nil sides 0, where JavaScript's `<` and `>` would coerce it ("5" to 5,
//! null to 0); so a product of it raises as Ruby's `nil * 3` does. On
//! real-blog's Article, run under tsx. The script names nil, NaN and
//! undefined apart, since JSON writes all three as `null`.

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

  #: (Integer, untyped) -> Integer
  def self.untyped_scaled(a, b)
    (a <=> b) * 3
  end

  #: (Integer, untyped) -> Integer?
  def self.untyped_order(a, b)
    a <=> b
  end

  #: (Integer, Integer?) -> Integer?
  def self.int_nil(a, b)
    a <=> b
  end

  #: (Integer?, Integer) -> Integer?
  def self.nil_int(a, b)
    a <=> b
  end

  #: (Integer?, Integer?) -> Integer?
  def self.nil_nil(a, b)
    a <=> b
  end

  #: (Integer, Integer?) -> Integer
  def self.int_nil_scaled(a, b)
    (a <=> b) * 3
  end
"#;

/// What the calls answer under CRuby, nil written as "nil", as JSON; a call
/// that raises, as its message. JavaScript's `undefined` is a nil too.
#[allow(dead_code)]
pub const SUMMARY: &str = r#"["nil","nil",-1,0,0,1,"nil",-3,"undefined method '*' for nil","nil","nil",0,-1,"nil","undefined method '*' for nil","nil","nil",-1,"undefined method '*' for nil",0,"nil"]"#;

/// The same calls, as the tsx script makes them, with nil, NaN and undefined named apart.
#[allow(dead_code)]
pub const CALLS: &str = "[Article.float_order(NaN, 1.0), Article.float_order(1.0, NaN), Article.float_order(1.5, 2.5), \
Article.float_order(2.5, 2.5), Article.int_order(2, 2), Article.int_order(3, 1), Article.mixed_order(1, NaN), \
Article.untyped_scaled(1, 2), (() => { try { return Article.untyped_scaled(1, \"foo\"); } catch (e) { return (e as Error).message; } })(), \
Article.int_nil(1, null), Article.nil_int(null, 1), Article.nil_nil(null, null), Article.int_nil(1, 2), \
Article.int_nil(1, undefined as any), \
(() => { try { return Article.int_nil_scaled(1, null); } catch (e) { return (e as Error).message; } })(), \
Article.untyped_order(1, \"5\"), Article.untyped_order(1, true), Article.untyped_order(1, 5), \
(() => { try { return Article.untyped_scaled(1, \"5\"); } catch (e) { return (e as Error).message; } })(), \
Article.nil_nil(undefined as any, null), Article.untyped_order(1, [2])]\
.map((x: unknown) => x === null ? \"nil\" : x === undefined ? \"undefined\" : Number.isNaN(x) ? \"NaN\" : x)";

pub fn overlay() -> Overlay {
    emit_and_run::real_blog().edit("app/models/article.rb", "class Article < ApplicationRecord\n", &format!("class Article < ApplicationRecord\n{MODEL}"))
}
