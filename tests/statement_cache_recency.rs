//! The CRuby shim's prepared-statement cache (runtime/spinel/db_cruby.rb,
//! `Db.prepare`) evicts its least recently used statement. Which statement
//! that is used to be recorded by deleting and re-inserting the SQL text in
//! the cache on every hit; it is now recorded apart, by the statement's
//! identity. The cache holds the same statements and evicts the same ones.

use std::path::Path;
use std::process::Command;

#[path = "support/prerequisite.rs"]
mod prerequisite;

/// What both tests share: a database, a cache capped at five statements, and
/// the cache as it has always behaved, written down as a list of SQL texts
/// with the least recently used first.
const SETUP: &str = r##"
require "tmpdir"
require "fileutils"
require_relative "runtime/ruby/active_record/connection_pool"
require_relative "runtime/spinel/db_cruby"
require_relative "runtime/ruby/active_record/errors"
Db.send(:remove_const, :STMT_CACHE_CAP)
Db.const_set(:STMT_CACHE_CAP, 5)
DIR = Dir.mktmpdir("rh-statement-recency")
at_exit { FileUtils.remove_entry(DIR) }
Db.configure(File.join(DIR, "t.sqlite3"), pool_size: 1)
Db.exec("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
12.times { |i| Db.exec("INSERT INTO t (v) VALUES ('row #{i}')") }
CONN = Db.current_dbh
cache = -> { CONN.instance_variable_get(:@rh_stmt_cache) || {} }
check = ->(cond, what) { raise "FAILED: #{what}" unless cond }
SQLS = Array.new(40) { |i| "SELECT v FROM t WHERE id = #{i + 1}" }
model = []   # SQL texts in the cache, least recently used first
held = {}    # SQL text => the handle of a reader left in the middle of its rows
# A read as the model sees it: the statement its eviction closes, if any.
note = lambda do |sql|
  if model.include?(sql)
    unless held.key?(sql)
      model.delete(sql)
      model << sql
    end
    next nil
  end
  victim = model.size >= 5 ? model.find { |text| !held.key?(text) } : nil
  model.delete(victim) unless victim.nil?
  model << sql
  victim
end
"##;

/// A long random run of reads, of readers left open and of releases keeps in
/// the cache exactly the statements the list predicts, closes the one it
/// evicts and no other. Holds before and after the change.
#[test]
fn the_cache_holds_and_evicts_what_it_always_has() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = format!(
        "{SETUP}{}",
        r##"
rng = Random.new(2009)
evictions = 0
past_the_cap = 0
6000.times do |step|
  sql = SQLS[rng.rand(SQLS.size)]
  action = rng.rand(10)
  if action < 7 || (action < 9 && (held.key?(sql) || held.size >= 7))
    before = cache.call.dup
    victim = note.call(sql)
    handle = Db.prepare(sql.dup)
    Db.step?(handle)
    Db.finalize(handle)
    unless victim.nil?
      evictions += 1
      check.(before[victim].closed?, "step #{step}: the statement evicted for #{sql.inspect} is not closed")
    end
  elsif action < 9
    note.call(sql)
    handle = Db.prepare(sql.dup)
    Db.step?(handle)
    held[sql] = handle
  elsif !held.empty?
    text = held.keys[rng.rand(held.size)]
    Db.finalize(held.delete(text))
  end
  now = cache.call
  past_the_cap += 1 if now.size > 5
  check.(now.keys.sort == model.sort, "step #{step}: the cache holds #{now.keys.sort.inspect}, the list #{model.sort.inspect}")
  check.(now.values.none?(&:closed?), "step #{step}: a cached statement is closed")
  check.(now.values.uniq.size == now.size, "step #{step}: one statement is cached under two texts")
end
held.each_value { |handle| Db.finalize(handle) }
check.(evictions > 300, "only #{evictions} evictions: the run did not exercise the cap")
check.(past_the_cap > 0, "the cache never went past its cap with every statement in use")
Db.close
puts "ALL OK #{evictions > 300}"
"##
    );
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", &script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK true"), "{stdout}");
}

/// A hit leaves the cache's own entries where they are: the SQL text is not
/// deleted and inserted again. The order of use is the recency table's,
/// which names every cached statement once, least recently used first.
#[test]
fn a_hit_records_recency_without_rewriting_the_cache_entry() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = format!(
        "{SETUP}{}",
        r##"
read = lambda do |sql|
  note.call(sql)
  handle = Db.prepare(sql.dup)
  Db.step?(handle)
  Db.finalize(handle)
end
SQLS.first(4).each { |sql| read.call(sql) }
order = cache.call.keys
count = 0
[SQLS[0], SQLS[2], SQLS[0], SQLS[3], SQLS[1]].each do |sql|
  read.call(sql)
  check.(cache.call.keys == order, "a hit on #{sql.inspect} moved the cache's entries: #{cache.call.keys.inspect}")
  count += 1
end
recency = -> { CONN.instance_variable_get(:@rh_stmt_recent) }
rng = Random.new(7)
3000.times do |step|
  sql = SQLS[rng.rand(SQLS.size)]
  if rng.rand(8).zero? && !held.key?(sql) && held.size < 7
    note.call(sql)
    handle = Db.prepare(sql.dup)
    Db.step?(handle)
    held[sql] = handle
  elsif rng.rand(8).zero? && !held.empty?
    text = held.keys[rng.rand(held.size)]
    Db.finalize(held.delete(text))
  else
    read.call(sql)
  end
  recent = recency.call
  check.(!recent.nil?, "step #{step}: no recency table on the connection")
  check.(recent.values == model, "step #{step}: the recency table says #{recent.values.inspect}, the list #{model.inspect}")
  check.(recent.all? { |statement, text| cache.call[text].equal?(statement) }, "step #{step}: the recency table names a statement the cache does not hold under that text")
  check.(recent.size == cache.call.size, "step #{step}: #{recent.size} statements in the recency table, #{cache.call.size} in the cache")
  count += 1
end
held.each_value { |handle| Db.finalize(handle) }
held.clear
# The recency table keeps the text as the statement was prepared with it: a caller that goes on writing to
# its String leaves the cached statement findable, and evictable, under that text.
prepared_as = "SELECT v FROM t WHERE id = 41"
text = +prepared_as
note.call(prepared_as)
handle = Db.prepare(text)
reused = handle[:stmt]
Db.step?(handle)
Db.finalize(handle)
text << " AND 1 = 1"
check.(recency.call[reused] == prepared_as, "the recency table follows the caller's String: #{recency.call[reused].inspect}")
check.(cache.call[prepared_as].equal?(reused), "the cache lost a statement after its text was written to")
count += 1
SQLS[20, 12].each { |sql| read.call(sql) }
check.(!model.include?(prepared_as) && reused.closed?, "the statement was not evicted and closed: in the list #{model.include?(prepared_as)}, closed #{reused.closed?}")
check.(!cache.call.key?(prepared_as) && !recency.call.key?(reused), "the evicted statement is still in a table")
count += 1
# A statement whose release fails leaves the cache, and the recency table with it.
sql = model.last
handle = Db.prepare(sql.dup)
failing = handle[:stmt]
failing.define_singleton_method(:reset!) { raise SQLite3::Exception, "release failed" }
begin
  Db.finalize(handle)
rescue StandardError
  nil
end
model.delete(sql)
check.(cache.call.keys.sort == model.sort, "after a failed release the cache holds #{cache.call.keys.sort.inspect}")
check.(!recency.call.key?(failing) && recency.call.values == model, "after a failed release the recency table says #{recency.call.values.inspect}")
count += 1
read.call(sql)
check.(recency.call.values == model && cache.call.keys.sort == model.sort, "read again, the text of a failed release is cached afresh")
count += 1
Db.close
puts "ALL OK #{count}"
"##
    );
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", &script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 3009"), "{stdout}");
}
