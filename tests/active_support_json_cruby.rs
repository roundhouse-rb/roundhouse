//! The CRuby overlay's ActiveSupport::JSON fast path
//! (ruby_overlay/runtime/active_support_json_cruby.rb) writes the same
//! document as the shared encoder for every value: the C generator's
//! strict mode for JSON-native values, the shared `jsonable` walk for
//! everything strict mode refuses.

use std::path::Path;
use std::process::Command;

#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn the_overlay_encoder_writes_the_shared_encoders_document() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require "json"
require_relative "runtime/ruby/json_builder"
require_relative "runtime/ruby/active_support_ext"
J = ActiveSupport::JSON
shared = J.method(:encode)
shared_plain = J.method(:encode_without_escape)
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/active_support_json_cruby"

class SafeText < String
  def to_json(*) = "\"SAFE\""
end
class Bag < Hash; end
bag = Bag.new; bag["k"] = "v"
values = [
  nil, true, 1, 2**70, 1.5, "<b>&</b> ", :sym, [], {},
  [{ "body" => "<div>x &amp; y</div>", "id" => 7, "n" => nil }, { a: :b }, [1, [2, [3]]]],
  { 1 => "integer key" }, [Float::NAN], [Float::INFINITY], [Time.utc(2026, 10, 22, 1, 2, 3, 456789)],
  [SafeText.new("<x>")], [bag],
]
values.each do |v|
  raise "encode differs for #{v.inspect}" unless J.encode(v) == shared.call(v)
  raise "encode_without_escape differs for #{v.inspect}" unless J.encode_without_escape(v) == shared_plain.call(v)
end
raise "escaping lost" unless J.encode(["<"]) == '["<"]'.gsub("<", "\\u003c")

# What neither path models still raises, the same error.
[Object.new, [Object.new]].each do |v|
  begin
    J.encode(v)
    raise "#{v.inspect} encoded"
  rescue ArgumentError
  end
end

# The fast path is taken: a native value never reaches the walk.
walks = 0
J.singleton_class.prepend(Module.new { define_method(:jsonable) { |x| walks += 1; super(x) } })
J.encode([{ "a" => "<b>", "n" => 1 }])
raise "a native value took the walk" unless walks == 0
J.encode([Time.utc(2026)])
raise "a Time skipped the walk" unless walks > 0
puts "ALL OK"
"##;
    let out = Command::new("ruby")
        .arg("-e")
        .arg(script)
        .current_dir(root)
        .output()
        .expect("ruby is on PATH");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("ALL OK"),
        "overlay JSON encoder failed\n=== stdout ===\n{stdout}\n=== stderr ===\n{stderr}"
    );
    assert!(out.status.success(), "driver exited {:?}", out.status.code());
}

/// A document with none of `<`, `>`, `&` in it is the answer as generated:
/// the escaping pass, which would copy it unchanged, is not run. What comes
/// back is still a String of the caller's own (unfrozen, another object at
/// every call) with the bytes and the encoding the escaping pass gives.
#[test]
fn a_document_with_nothing_to_escape_is_not_copied() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require "json"
require_relative "runtime/ruby/json_builder"
require_relative "runtime/ruby/active_support_ext"
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/active_support_json_cruby"
J = ActiveSupport::JSON
# The answer as it was written: the generated document through the escaping pass, every time.
reference = ->(v) { JsonBuilder.escape_html_entities(J.encode_without_escape(v)) }
plain = [nil, true, false, 0, -7, 2**70, 1.5, "", "plain", "é ü 日本", "quote \" and \\ backslash", "line\nbreak", :sym, [], {},
         [1, "two", nil, [3.5, { "k" => "v" }]], { "id" => 7, "name" => "Ada", "tags" => %w[a b] }, { a: :b },
         [Time.utc(2026, 10, 22, 1, 2, 3)], [Float::NAN], "   "]
escaped = ["<", ">", "&", "<b>&</b>", ["a<b"], { "k" => "x & y" }, { "<k>" => 1 }, "é<", [Time.utc(2026), "<"], :"a&b"]
count = 0
(plain + escaped).each do |v|
  want = reference.(v)
  got = J.encode(v)
  raise "FAILED: #{v.inspect} encodes as #{got.inspect}, was #{want.inspect}" unless got == want
  raise "FAILED: #{v.inspect} comes back in #{got.encoding}, was #{want.encoding}" unless got.encoding == want.encoding
  raise "FAILED: #{v.inspect} comes back frozen" if got.frozen?
  raise "FAILED: #{v.inspect} comes back as the same object twice" if got.equal?(J.encode(v))
  got << "!" # the caller's own String: a write to it reaches nothing else
  raise "FAILED: #{v.inspect} was changed by a write to an earlier answer" unless J.encode(v) == want
  count += 1
end
escaped.each do |v|
  raise "FAILED: #{v.inspect} kept a character to escape" if J.encode(v).match?(/[<>&]/)
  count += 1
end
# Fewer objects per call when there is nothing to escape (the copy, and what `gsub` allocates to make
# it: two on Ruby 3.4 and 4.0); as many as before when there is.
per_call = lambda do |encoder, v|
  200.times { encoder.(v) }
  before = GC.stat(:total_allocated_objects)
  1000.times { encoder.(v) }
  (GC.stat(:total_allocated_objects) - before).fdiv(1000)
end
now = ->(v) { J.encode(v) }
[{ "id" => 7, "name" => "Ada" }, "plain", [1, 2, 3]].each do |v|
  saved = per_call.(reference, v) - per_call.(now, v)
  raise "FAILED: #{v.inspect} saves #{saved} objects per call, want at least 1" unless saved > 0.8
  count += 1
end
[{ "k" => "x & y" }, "<b>"].each do |v|
  saved = per_call.(reference, v) - per_call.(now, v)
  raise "FAILED: #{v.inspect}, which has something to escape, differs by #{saved} objects per call" unless saved.abs < 0.2
  count += 1
end
puts "ALL OK #{count}"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 46"), "{stdout}");
}
