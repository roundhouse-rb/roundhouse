//! `ActionDispatch::Request#ssl?` on CRuby answers what it answered for
//! every mix of `HTTPS`, `X-Forwarded-Proto` and `rack.url_scheme`, and a
//! request with no proxy header and a plain `http` or `https` scheme, the
//! common one, is answered from the env's own Strings without building any.

#[path = "support/emit_and_run.rs"]
mod emit_and_run;

#[test]
fn ssl_answers_as_before_for_every_env_and_builds_nothing_for_a_plain_one() {
    let run = emit_and_run::real_blog().run_ruby(
        r##"
# The answer as it was computed before any shortcut.
reference = lambda do |env|
  next true if env["HTTPS"] == "on"
  forwarded = env["HTTP_X_FORWARDED_PROTO"].to_s.split(",").first.to_s.strip.downcase
  next forwarded == "https" unless forwarded.empty?
  env["rack.url_scheme"].to_s.downcase == "https"
end
request = lambda do |env|
  full = { "REQUEST_METHOD" => "GET", "PATH_INFO" => "/", "QUERY_STRING" => "", "HTTP_HOST" => "example.test",
           "rack.input" => StringIO.new("") }.merge(env)
  ActionDispatch::Request.new(full)
end
https = [:absent, nil, "on", "off", "ON", ""]
forwarded = [:absent, nil, "", " ", ",", "https", "http", "HTTPS", "Https", " https ", "https,http", "http,https",
             "https, http", ",https", " ,https", "wss", "ws", "https ,", "httpsx", "httpſ"]
schemes = [:absent, nil, "http", "https", "HTTPS", "Https", "HTTP", "", "ws", "wss", " https", "httpsx", "httpſ",
           "HTTPſ", :https]
count = 0
https.each do |h|
  forwarded.each do |f|
    schemes.each do |s|
      env = {}
      env["HTTPS"] = h unless h == :absent
      env["HTTP_X_FORWARDED_PROTO"] = f unless f == :absent
      env["rack.url_scheme"] = s unless s == :absent
      req = request.(env)
      got = req.ssl?
      raise "FAILED: #{env.inspect} answers #{got.inspect}, want #{reference.(env).inspect}" unless got == reference.(env)
      raise "FAILED: #{env.inspect} protocol #{req.protocol}" unless req.protocol == (got ? "https://" : "http://")
      count += 1
    end
  end
end
raise "FAILED: the table has #{count} rows" unless count == 1800

[["http", false], ["https", true]].each do |scheme, want|
  req = request.("rack.url_scheme" => scheme)
  raise "FAILED: plain #{scheme}" unless req.ssl? == want
  1000.times { req.ssl? }
  before = GC.stat(:total_allocated_objects)
  1000.times { req.ssl? }
  per_call = (GC.stat(:total_allocated_objects) - before).fdiv(1000)
  raise "FAILED: ssl? on a plain #{scheme} request allocates #{per_call} objects per call, want 0" unless per_call < 0.5
end
puts "ALL OK"
"##,
    );
    run.assert_passes();
    assert!(run.stdout.contains("ALL OK"), "{}", run.stdout);
}
