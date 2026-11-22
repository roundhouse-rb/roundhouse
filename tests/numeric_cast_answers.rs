//! `ActiveSupport.cast_integer`, `cast_float` and `cast_decimal` (ActiveModel's
//! Integer, Float and Decimal casts) test the value's own number class first,
//! before nil, `true` and `false`. A number is none of those, so no answer
//! moves: these tables were recorded from the three methods as they were
//! before the reorder. They pin every class of value, the Float edges (NaN,
//! the infinities, negative zero), the String forms the 16-byte `to_i` and
//! `to_f` read, the values Rails' Float cast raises on, and that a number
//! already of the column's class comes back as the very object given.

use std::path::Path;
use std::process::Command;

#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn numeric_casts_answer_every_class_as_before_the_reorder() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require_relative "runtime/ruby/active_support_ext"
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/active_support_duration"
big = 2**70
inf = Float::INFINITY
d = ActiveSupport::Duration
values = [0, -5, 7, big, -big, nil, true, false, 1.9, -1.9, 0.0, -0.0, Float::NAN, inf, -inf,
  "12", "12abc", "", " ", 0xA0.chr(Encoding::UTF_8), "\t\n", "0x1A", "-7", "+8", "1_0", "1__0", "12345678901234567890", "abc", " 42 ",
  "4" + 0xE9.chr(Encoding::UTF_8) + "2",
  Rational(7, 2), Rational(-7, 2), Complex(3, 0), :sym, :"", [], [1], {}, { a: 1 }, Object.new, Time.at(5), Time.at(-5.5),
  d.new(90), d.minutes(2), d.new(2.5)]
want = {
  cast_integer: [0, -5, 7, big, -big, nil, 1, 0, 1, -1, 0, 0, nil, nil, nil,
    12, 12, nil, nil, nil, nil, 0, -7, 8, 10, 1, 1234567890123456, 0, 42, 4,
    3, -3, 3, nil, nil, nil, nil, nil, nil, nil, 5, -6,
    90, 120, 2],
  cast_float: [0.0, -5.0, 7.0, big.to_f, -big.to_f, nil, 1.0, 0.0, 1.9, -1.9, 0.0, -0.0, :nan, inf, -inf,
    12.0, 12.0, nil, nil, nil, nil, 0.0, -7.0, 8.0, 10.0, 1.0, 12345678901234567890.0, 0.0, 42.0, 4.0,
    3.5, -3.5, 3.0, :raise, nil, nil, :raise, nil, :raise, :raise, 5.0, -5.5,
    90.0, 120.0, 2.5],
  cast_decimal: [0.0, -5.0, 7.0, big.to_f, -big.to_f, nil, 1.0, 0.0, 1.9, -1.9, 0.0, -0.0, :nan, inf, -inf,
    12.0, 12.0, nil, nil, nil, nil, 0.0, -7.0, 8.0, 10.0, 1.0, 12345678901234567890.0, 0.0, 42.0, 4.0,
    3.5, -3.5, 3.0, 0.0, nil, nil, 0.0, nil, 0.0, 0.0, 1970.0, 1969.0,
    90.0, 120.0, 2.5],
}
count = 0
want.each do |m, answers|
  raise "FAILED: #{m} has #{answers.size} answers for #{values.size} values" unless answers.size == values.size
  values.zip(answers).each do |value, w|
    got = begin
      ActiveSupport.public_send(m, value)
    rescue NoMethodError
      :raise
    end
    ok = case w
         when :nan then got.is_a?(Float) && got.nan?
         when :raise then got == :raise
         else got.class == w.class && got == w
         end
    raise "FAILED: #{m}(#{value.inspect}) => #{got.inspect}, want #{w.inspect}" unless ok
    own = m == :cast_integer ? value.is_a?(Integer) : value.is_a?(Float)
    raise "FAILED: #{m}(#{value.inspect}) came back as another object" if own && !got.equal?(value)
    count += 1
  end
end
puts "ALL OK #{count}"
"##;
    // TZ is pinned: the decimal cast of a Time reads the year off `Time#to_s`, which is local time.
    let Some(output) =
        prerequisite::output_of(Command::new("ruby").args(["-e", script]).env("TZ", "UTC").current_dir(root))
    else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 135"), "{stdout}");
}
