//! `Relation#column_bound_value` (a hash condition's value through its
//! column's type) and `ActiveRecord::Type::Integer#cast` return an Integer
//! before comparing the value with `true` and `false`. An Integer is never
//! either, so no answer moves: the tables were recorded from both methods as
//! they were before the reorder (every column kind: an integer column, a
//! string column, a column of another table, a missing one). An Integer comes
//! back as the very object given. Where YJIT runs, an Integer no longer
//! leaves the compiled code: `Integer#==` against `true` is compiled only for
//! two fixnums, so the old order was a side exit per value, and every
//! `where(id: ids)` element passes through `column_bound_value`.

use std::path::Path;
use std::process::Command;

#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn integer_values_bind_and_cast_as_before_without_leaving_yjit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require_relative "runtime/ruby/active_record/relation"
require_relative "runtime/ruby/active_record/types"
big = 2**70
values = [0, 7, -1, big, -big, true, false, nil, "1", "0", "t", "", 1.5, 0.0, :x, [1], { a: 1 }]
model = Class.new { def self.schema_column_types = { id: :integer, flag: :integer, name: :string } }
rel = ActiveRecord::Relation.allocate
rel.instance_variable_set(:@model, model)
as_given = [0, 7, -1, big, -big, true, false, nil, "1", "0", "t", "", 1.5, 0.0, :x, [1], { a: 1 }]
cast_int = [0, 7, -1, big, -big, 1, 0, nil, "1", "0", "t", "", 1.5, 0.0, :x, [1], { a: 1 }]
want = { "id" => cast_int, "flag" => cast_int, "name" => as_given, "items.flag" => as_given, "missing" => as_given }
count = 0
want.each do |col, answers|
  values.zip(answers).each do |value, w|
    got = rel.column_bound_value(col, value)
    raise "FAILED: column_bound_value(#{col.inspect}, #{value.inspect}) => #{got.inspect}, want #{w.inspect}" unless got.class == w.class && got == w
    raise "FAILED: column_bound_value(#{col.inspect}, #{value.inspect}) came back as another object" if value.is_a?(Integer) && !got.equal?(value)
    count += 1
  end
end
caster = ActiveRecord::Type::Integer.new
[0, 7, -1, big, -big, 1, 0, nil, 1, 0, 0, nil, 1, 0, nil, nil, nil].each_with_index do |w, i|
  got = caster.cast(values[i])
  raise "FAILED: Type::Integer#cast(#{values[i].inspect}) => #{got.inspect}, want #{w.inspect}" unless got.class == w.class && got == w
  raise "FAILED: Type::Integer#cast(#{values[i].inspect}) came back as another object" if values[i].is_a?(Integer) && !got.equal?(values[i])
  count += 1
end

if defined?(RubyVM::YJIT) && RubyVM::YJIT.respond_to?(:enable) && RubyVM::YJIT.enable(stats: :quiet) && RubyVM::YJIT.stats_enabled?
  run =-> { [1, 2, 3].each { |v| rel.column_bound_value("id", v); rel.column_bound_value("name", v); caster.cast(v) } }
  2000.times { run.() }
  # The comparison's own counter: the process-wide count also moves when
  # an interrupt (a GC step, a signal) lands inside compiled code.
  # `fetch`: a counter that is not there must not read as zero.
  before = RubyVM::YJIT.runtime_stats.fetch(:exit_opt_eq)
  500.times { run.() }
  exits = RubyVM::YJIT.runtime_stats.fetch(:exit_opt_eq) - before
  raise "FAILED: YJIT opt_eq exits in 4500 calls with Integer values: #{exits}" unless exits == 0
  puts "YJIT checked"
end
puts "ALL OK #{count}"
"##;
    // YJIT is switched on by the script, with its counters: one already on
    // from the environment would leave them off.
    let Some(output) = prerequisite::output_of(
        Command::new("ruby").args(["-e", script]).env_remove("RUBY_YJIT_ENABLE").env_remove("RUBYOPT").current_dir(root),
    ) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 102"), "{stdout}");
    if !stdout.contains("YJIT checked") {
        prerequisite::missing("a CRuby with YJIT and its counters (RubyVM::YJIT.enable(stats:)): the side-exit half did not run");
    }
}
