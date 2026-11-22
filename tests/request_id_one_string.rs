//! A request with no id of its own gets a fresh UUID. On CRuby and JRuby
//! (ruby_overlay/runtime/request_id_cruby.rb) it is written as one String
//! from the sixteen bytes `SecureRandom.uuid` would format: the same id for
//! the same bytes, in the same encoding, with fewer objects. A request that
//! brings an id is answered by the shared method, and so is one made while a
//! test stubs `SecureRandom.uuid` through the emitted stub slot.

use std::path::Path;
use std::process::Command;

#[path = "support/emit_and_run.rs"]
mod emit_and_run;
#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn a_fresh_request_id_is_the_librarys_uuid_for_the_same_bytes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // The script copies the stub slot the Ruby tree is emitted with: if that changes, so must the copy.
    let project = std::fs::read_to_string(root.join("src/project.rs")).expect("src/project.rs");
    assert!(
        project.contains("STUB_UUID_ON = [ false ]") && project.contains("return STUB_UUID[0] if STUB_UUID_ON[0]"),
        "the stub slot this test copies is not the one src/project.rs emits any more"
    );
    let script = r##"
require "securerandom"
# The stub slot, as the emitted tree defines it (runtime/secure_random_stub.rb).
module SecureRandom
  STUB_UUID_ON = [ false ]
  STUB_UUID = [ "" ]
  class << self
    alias_method :uuid_without_stub, :uuid
    def uuid
      return STUB_UUID[0] if STUB_UUID_ON[0]
      uuid_without_stub
    end
  end
end
require_relative "runtime/ruby/action_dispatch/request_id"
R = ActionDispatch::RequestId
shared = R.method(:make)
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/request_id_cruby"

# The sixteen bytes both ask for, fixed while `fixed` is set; every call is counted.
fixed = nil
asked = []
SecureRandom.singleton_class.prepend(Module.new {
  define_method(:random_bytes) { |n = nil|
    asked << n
    fixed ? fixed.dup : super(n)
  }
})
count = 0
patterns = (0..255).map { |b| b.chr * 16 }
seeded = Random.new(2009)
300.times { patterns << seeded.bytes(16) }
patterns.each do |bytes|
  fixed = bytes.b
  want = SecureRandom.uuid
  asked.clear
  got = R.make("")
  raise "FAILED: #{bytes.unpack1('H*')} gives #{got.inspect}, the library gives #{want.inspect}" unless got == want
  raise "FAILED: the id comes back in #{got.encoding}, the library's in #{want.encoding}" unless got.encoding == want.encoding
  raise "FAILED: the id comes back frozen" if got.frozen?
  raise "FAILED: the random bytes were asked #{asked.inspect}, want once for 16" unless asked == [16]
  raise "FAILED: the fixed bytes were written to" unless fixed == bytes.b
  count += 1
end
fixed = nil

# With no id, or a blank one, a fresh id each time; the library's shape.
shape = /\A\h{8}-\h{4}-4\h{3}-[89ab]\h{3}-\h{12}\z/
["", " ", "\t\r\n ", "\v\f"].each do |blank|
  ids = Array.new(500) { R.make(blank) }
  raise "FAILED: #{blank.inspect} repeats an id" unless ids.uniq.size == ids.size
  raise "FAILED: #{blank.inspect} gives #{ids.grep_v(shape).first.inspect}" unless ids.all?(shape)
  raise "FAILED: #{blank.inspect} hands out one object twice" unless ids.map(&:object_id).uniq.size == ids.size
  count += 1
end

# A request that brings an id: the shared method's answer, and no random bytes.
["abc-123", "a b<c>", "x@y_z", "é-ü", "-" * 300, "a", "0", " a ", " ", "id\n"].each do |value|
  asked.clear
  got = R.make(value)
  want = shared.call(value)
  raise "FAILED: #{value.inspect} is cleaned to #{got.inspect}, the shared method gives #{want.inspect}" unless got == want && got.encoding == want.encoding
  raise "FAILED: #{value.inspect} asked for random bytes" unless asked.empty?
  count += 1
end

# While a test stubs SecureRandom.uuid: the stubbed id, from the shared method, and no random bytes.
SecureRandom::STUB_UUID[0] = "stubbed-id"
SecureRandom::STUB_UUID_ON[0] = true
asked.clear
got = R.make("")
raise "FAILED: with the stub on the id is #{got.inspect}, the shared method gives #{shared.call("").inspect}" unless got == "stubbed-id" && shared.call("") == "stubbed-id"
raise "FAILED: with the stub on, random bytes were asked" unless asked.empty?
SecureRandom::STUB_UUID_ON[0] = false
raise "FAILED: with the stub off the id is still the stubbed one" if R.make("") == "stubbed-id"
count += 1

# At most three objects for an id, the sixteen bytes and the String that is the id among them.
per_call = lambda do |make|
  200.times { make.call }
  before = GC.stat(:total_allocated_objects)
  1000.times { make.call }
  (GC.stat(:total_allocated_objects) - before).fdiv(1000)
end
library = per_call.(-> { SecureRandom.uuid })
ours = per_call.(-> { R.make("") })
raise "FAILED: a fresh id allocates #{ours} objects, want at most 3 (the library's allocates #{library})" unless ours <= 3.5
count += 1
puts "ALL OK #{count}"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 572"), "{stdout}");
}

/// The emitted tree loads the overlay after the shared `RequestId`, and the stub slot it is emitted with is
/// the one the overlay asks.
#[test]
fn emitted_ruby_boot_loads_the_one_string_request_id() {
    emit_and_run::empty_app()
        .write("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n")
        .run_ruby(r##"
raise "the overlay is not loaded" unless defined?(ActionDispatch::RequestIdOneString) &&
  ActionDispatch::RequestId.singleton_class.ancestors.first == ActionDispatch::RequestIdOneString
shape = /\A\h{8}-\h{4}-4\h{3}-[89ab]\h{3}-\h{12}\z/
id = ActionDispatch::RequestId.make("")
raise "a fresh id is #{id.inspect}" unless id.match?(shape)
require File.expand_path("runtime/secure_random_stub", Dir.pwd) unless SecureRandom.respond_to?(:stub_uuid)
SecureRandom.stub_uuid("stubbed-id")
answered = ActionDispatch::RequestId.make("")
raise "the stub is not answered: #{answered.inspect}" unless answered == "stubbed-id"
SecureRandom.clear_secure_random_stubs
raise "the stub outlived its clearing" unless ActionDispatch::RequestId.make("").match?(shape)
puts "ALL OK"
"##)
        .assert_passes();
}
