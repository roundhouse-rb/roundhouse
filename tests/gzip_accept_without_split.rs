//! `GzipCache.accepts_gzip?` reads `Accept-Encoding` on every response.
//! The answer is the one the full parse gives (codings split on commas,
//! any case, `gzip` or `x-gzip`, refused at q=0) for every header a server
//! hands over (a binary String, any bytes) and every valid String in an
//! ASCII-compatible encoding, and the two headers a server sees most are
//! answered without splitting: one that starts with a bare `gzip` (every
//! browser's) and one that never spells gzip at all.

use std::path::Path;
use std::process::Command;

#[path = "support/prerequisite.rs"]
mod prerequisite;

#[test]
fn accepts_gzip_matches_the_full_parse_and_skips_it_for_the_common_headers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = r##"
require_relative "runtime/spinel/scaffold/ruby_overlay/runtime/gzip_cache"
# The full parse, as it was before any shortcut.
reference = lambda do |accept|
  accept.to_s.downcase.split(",").any? { |part|
    coding, *params = part.strip.split(";")
    next false unless coding == "gzip" || coding == "x-gzip"
    q = "1"
    params.each { |p|
      k, v = p.strip.split("=", 2)
      q = v.to_s if k == "q"
    }
    q.to_f > 0.0
  }
end
codings = ["gzip", "GZIP", "Gzip", "x-gzip", "X-GZIP", "identity", "br", "deflate", "zstd", "*", "",
           "gzip;q=0", "gzip;q=0.0", "gzip;q=0.5", "gzip; q=0", "gzip ;q=0", "GZIP;Q=0", "gzip;q=1;q=0",
           "x-gzip;q=0", "gzipped", "ungzip", "not-gzip", "gzip2", " gzip", "gzip ", "gzip;", ";gzip",
           "g zip", "gz", "zip", "identity;q=1", "*;q=0", "gzip;level=9", "gzip;q", "gzip;q="]
headers = [nil]
codings.each { |a| headers << a }
codings.each { |a| codings.each { |b| headers << "#{a},#{b}" << "#{a}, #{b}" } }
codings.first(12).each { |a| codings.first(12).each { |b| codings.first(12).each { |c| headers << "#{a}, #{b},#{c}" } } }
headers << "gzip, deflate, br, zstd" << "gzip, deflate, br" << "gzip,deflate" << "deflate, gzip, br, zstd" <<
  "gzip;q=1.0,deflate;q=0.6,identity;q=0.3" << "gzip," << ",gzip" << ",,gzip,," << "gzip,;q=0" << "," << " "
headers.each do |h|
  got = GzipCache.accepts_gzip?(h)
  raise "FAILED: #{h.inspect} answers #{got.inspect}, the full parse #{reference.(h).inspect}" unless got == reference.(h)
end
raise "FAILED: the table has no refused gzip" unless headers.any? { |h| h.to_s.include?("gzip") && !reference.(h) }

# Binary strings, as a server hands headers over, with bytes above ASCII too.
high = 255.chr
["gzip, deflate, br, zstd", "identity", "GZIP", "gzip;q=0, br", "gzip,#{high}", "#{high}gzip", "#{high},gzip",
 "identity#{high}", "g#{high}zip", "GZIP#{high};q=0, x-gzip"].each do |h|
  b = h.b
  raise "FAILED: binary #{h.inspect}" unless GzipCache.accepts_gzip?(b) == reference.(b)
end
# Valid non-ASCII text: only ASCII letters spell gzip, in either reading.
["gzİp", "GZİP, br", "Kgzip", "gzip, é", "gſzip", "é, GZIP"].each do |h|
  raise "FAILED: #{h.inspect}" unless GzipCache.accepts_gzip?(h) == reference.(h)
end

{ "gzip, deflate, br, zstd" => true, "gzip" => true, "identity" => false, "" => false, "br, deflate" => false }.each do |h, want|
  1000.times { GzipCache.accepts_gzip?(h) }
  before = GC.stat(:total_allocated_objects)
  1000.times { GzipCache.accepts_gzip?(h) }
  per_call = (GC.stat(:total_allocated_objects) - before).fdiv(1000)
  raise "FAILED: #{h.inspect} answers #{GzipCache.accepts_gzip?(h)}" unless GzipCache.accepts_gzip?(h) == want
  raise "FAILED: #{h.inspect} allocates #{per_call} objects per call, want 0" unless per_call < 0.5
end
puts "ALL OK #{headers.size}"
"##;
    let Some(output) = prerequisite::output_of(Command::new("ruby").args(["-e", script]).current_dir(root)) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("ALL OK 4225"), "{stdout}");
}
