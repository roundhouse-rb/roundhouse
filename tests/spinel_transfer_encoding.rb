# Only supported, unambiguous transfer coding reaches the app.
require_relative "tep_server_harness"

FIELDS = [
  "Transfer-Encoding: chunked\r\n",
  "Transfer-Encoding: gzip\r\n",
  "Transfer-Encoding: identity\r\n",
  "Transfer-Encoding: chunked, gzip\r\n",
  "Transfer-Encoding : chunked\r\n",
  "Transfer-Encoding\t: chunked\r\n",
  "Transfer-Encoding: chunked\r\n gzip\r\n",
  "Transfer-Encoding: chunked\r\nContent-Length: nope\r\n",
  "Transfer-Encoding: chunked\r\nContent-Length: 9999999999999999999999999\r\n",
  "Transfer-Encoding: gzip, chunked\r\n",
  "Transfer-Encoding:\r\n",
  "tRaNsFeR-EnCoDiNg: CHUNKED\r\n",
  "Transfer-Encoding: chunked\r\nContent-Length: 2\r\n",
  "Transfer-Encoding: chunked\r\nTransfer-Encoding:\r\n"
].freeze

SERVERS.each_key do |server|
  ["1.0", "1.1"].each do |version|
    FIELDS.each do |fields|
      bytes = "POST /posts HTTP/#{version}\r\nHost: localhost\r\n#{fields}\r\n" \
              "2\r\né\r\n0\r\n\r\n" + "x" * 8192
      result = serve(server, bytes)
      status, recvs, bodies, raised, keep_alive = result
      accepted = version == "1.1" && ["Transfer-Encoding: chunked\r\n", "tRaNsFeR-EnCoDiNg: CHUNKED\r\n"].include?(fields)
      unsupported = version == "1.1" && ["Transfer-Encoding: identity\r\n", "Transfer-Encoding: gzip, chunked\r\n"].include?(fields)
      expected = accepted ? 200 : (unsupported ? 501 : 400)
      check(
        "#{server}: #{fields.inspect} answers #{expected} with exact body framing",
        raised.nil? && status == expected && recvs == 1 && bodies.map(&:b) == (accepted ? ["é".b] : []),
        describe(*result)
      )
      check(
        "#{server}: only accepted chunked coding keeps the connection",
        raised.nil? && keep_alive == accepted && (accepted || Sock.wire.out.include?("Connection: close\r\n")),
        describe(*result)
      )
    end
  end
  result = serve(server, post(2, "é".b))
  status, _recvs, bodies, raised, keep_alive = result
  check(
    "#{server}: an ordinary Content-Length body still serves",
    raised.nil? && status == 200 && keep_alive && bodies.map(&:b) == ["é".b],
    describe(*result)
  )
end

puts "#{CHECKS.count(true)}/#{CHECKS.length} checks pass"
puts "done"
