//! The CRuby overlay's parse_db_time keeps the UTC instant of a storage-form
//! timestamp per thread: every call still answers what a first parse gives
//! (a fresh thread has nothing kept), in every zone the app switches to, as
//! a new Time each time, and a caller that moves its Time to another zone
//! changes nothing for the next call.

use std::path::Path;
use std::process::Command;

#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn remembered_db_times_answer_a_first_parse_in_every_zone() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require_relative "runtime/ruby/active_support_ext"
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/active_support_time_parsing"
check = ->(what, cond) { raise "FAILED: #{what}" unless cond }
outcome = ->(&blk) { begin; t = blk.call; t.nil? ? [:nil] : [t.class, t.iso8601(6), t.utc_offset, t.utc?, t.frozen?, t.zone]; rescue => e; [e.class, e.message]; end }
# A first parse: a thread with nothing kept.
fresh = ->(str) { Thread.new { outcome.call { ActiveSupport.parse_db_time(str) } }.value }

strings = ["2026-01-02 03:04:05", "2026-01-02 03:04:05.123456", "2026-07-02T03:04:05.5", "2026-01-02 03:04:05.123456789",
           "2024-02-29 23:59:59.999999", "1999-12-31 23:59:60", "2026-13-01 00:00:00", "2026-01-02 03:04:05Z",
           "2026-01-02 03:04:05+09:00", "not a time", "", nil, "2026-01-02 03:04:05".b, "2026-01-02 03:04:05".freeze]
zones = ["UTC", "Asia/Tokyo", "America/New_York", "Europe/London"]
2.times do
  zones.each do |zone|
    ActiveSupport.set_default_zone(zone)
    strings.each do |s|
      want = fresh.call(s)
      first = outcome.call { ActiveSupport.parse_db_time(s) }
      again = outcome.call { ActiveSupport.parse_db_time(s) }
      check.("#{zone} #{s.inspect}: #{first.inspect} vs #{want.inspect}", first == want && again == want)
    end
  end
end
ActiveSupport.set_default_zone("Asia/Tokyo")
s = "2026-01-02 03:04:05.123456"
a = ActiveSupport.parse_db_time(s)
b = ActiveSupport.parse_db_time(s)
check.("a new Time each call", !a.equal?(b) && !a.frozen?)
a.localtime("+05:00")
check.("a caller moving its Time does not move the next", outcome.call { ActiveSupport.parse_db_time(s) } == fresh.call(s))
k = +"2026-01-02 03:04:05"
ActiveSupport.parse_db_time(k)
k << ".5"
check.("a mutated key string", outcome.call { ActiveSupport.parse_db_time(k) } == fresh.call(k))
check.("kept instants frozen", Thread.current.thread_variable_get(:rh_db_times).values.all?(&:frozen?))
5000.times { |i| ActiveSupport.parse_db_time(format("2026-01-01 00:00:%02d.%06d", i % 60, i)) }
check.("cap held", Thread.current.thread_variable_get(:rh_db_times).size <= ActiveSupport::DB_TIMES_CAP)
check.("after the cap", outcome.call { ActiveSupport.parse_db_time(s) } == fresh.call(s))
puts "ALL OK"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    assert!(output.status.success(), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("ALL OK"));
}
