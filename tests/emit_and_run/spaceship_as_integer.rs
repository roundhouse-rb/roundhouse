//! `<=>` answers -1, 0 or 1, and programs use the answer as the Integer it
//! is: a reversed comparison (`(a <=> b) * -1`), a weighted sum, its
//! negation, a `sort` block, and a condition, where 0 is true. Integer,
//! String and Symbol receivers; the analysis typed the answer `bool` and
//! refused the arithmetic. Float answers nil against NaN, and `true` against
//! `false` answers nil.

use super::emit_and_run;

const MODEL: &str = r#"
  #: (Integer, Integer) -> Integer
  def self.reversed(a, b)
    (a <=> b) * -1
  end

  #: (String, String) -> Integer
  def self.weighted(a, b)
    total = 0
    [1, 2].each { |k| total += (a <=> b) * k }
    total
  end

  #: (Symbol, Symbol) -> Integer
  def self.negated(a, b)
    -(a <=> b)
  end

  #: (Array[Integer]) -> Array[Integer]
  def self.descending(xs)
    xs.sort { |x, y| (x <=> y) * -1 }
  end

  #: (Integer, Integer) -> String
  def self.answered(a, b)
    (a <=> b) ? "t" : "f"
  end

  #: (Float, Float) -> Integer?
  def self.floats(a, b)
    a <=> b
  end

  #: (Float, Float) -> Integer
  def self.floats_reversed(a, b)
    (a <=> b) * -1
  end

  #: (bool, bool) -> Integer?
  def self.booleans(a, b)
    a <=> b
  end
"#;

const SCRIPT: &str = r#"got = [Article.reversed(1, 2), Article.reversed(2, 2), Article.reversed(3, 2)]
raise got.inspect unless got == [1, 0, -1]
got = [Article.weighted("b", "a"), Article.weighted("a", "b"), Article.weighted("a", "a")]
raise got.inspect unless got == [3, -3, 0]
got = [Article.negated(:a, :b), Article.negated(:b, :a)]
raise got.inspect unless got == [1, -1]
got = Article.descending([3, 1, 2])
raise got.inspect unless got == [3, 2, 1]
got = [Article.answered(0, 0), Article.answered(1, 2)]
raise got.inspect unless got == ["t", "t"]
nan = 0.0 / 0.0
got = [Article.floats(1.5, 2.5), Article.floats(2.5, 2.5), Article.floats(nan, 1.0)]
raise got.inspect unless got == [-1, 0, nil]
got = Article.floats_reversed(1.5, 2.5)
raise got.inspect unless got == 1
got = [Article.booleans(true, true), Article.booleans(true, false)]
raise got.inspect unless got == [0, nil]
puts "spaceship as integer passed"
"#;

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog().edit("app/models/article.rb", "class Article < ApplicationRecord\n", &format!("class Article < ApplicationRecord\n{MODEL}"))
}

#[test]
fn a_comparison_answers_an_integer_the_program_can_use() {
    let run = app().run_ruby(SCRIPT);
    run.assert_passes();
    assert!(run.stdout.contains("spaceship as integer passed"), "{}", run.stdout);
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn a_comparison_answers_an_integer_the_program_can_use_natively() {
    let run = app().run_spinel(SCRIPT);
    run.assert_passes();
    assert!(run.stdout.contains("spaceship as integer passed"), "{}", run.stdout);
}
