# The same literal body/framing results are checked by the Puma control.
ENV["TEP_MAX_BODY_BYTES"] = "1024"
require_relative "tep_server_harness"

class RecordingApp
  attr_reader :requests
  alias reset_without_requests reset
  def reset
    reset_without_requests
    @requests = []
  end
  def dispatch(req, res)
    @bodies << req.raw_body.b
    @requests << [req.path, req.req_params["title"], req.json_body_params["title"], req.req_headers.dup]
    res.body = req.req_params["title"]
  end
end

def chunked(parts, trailers = "", extension = "")
  parts.map { |part| "#{part.bytesize.to_s(16)}#{extension}\r\n".b + part.b + "\r\n" }.join.b + "0\r\n#{trailers}\r\n".b
end

def request(body, fields = "Transfer-Encoding: chunked\r\n", content_type = "application/x-www-form-urlencoded")
  "POST /posts HTTP/1.1\r\nHost: localhost\r\nContent-Type: #{content_type}\r\n#{fields}\r\n".b + body.b
end

SERVERS.each_key do |server|
  [1, 2, 7, 4096, 65535].each do |read_size|
    body = chunked(["title=", "hello"], "X-Trace: ignored\r\n", ';name="a;b";flag')
    result = serve(server, request(body), chunk: read_size)
    status, _recvs, bodies, raised, keep_alive = result
    check("#{server}: decoded form with extensions/trailers, read=#{read_size}",
      raised.nil? && status == 200 && keep_alive && bodies == ["title=hello"] && APP.requests[0][1] == "hello", describe(*result))
    headers = APP.requests.empty? ? {} : APP.requests[0][3]
    check("#{server}: decoded env framing, read=#{read_size}", headers["content-length"] == "11" &&
      !headers.key?("transfer-encoding") && !headers.key?("x-trace"), headers.inspect)
  end

  [
    ["empty", chunked([]), "", ""],
    ["UTF-8", chunked(["title=%C3", "%A9"]), "title=%C3%A9", "é"],
    ["one-byte chunks", chunked("title=hello".chars), "title=hello", "hello"],
    ["ignored framing trailer", chunked(["title=hello"], "Content-Length: 999\r\n"), "title=hello", "hello"],
  ].each do |label, encoded, decoded, title|
    result = serve(server, request(encoded), chunk: 1)
    check("#{server}: #{label}", result[0] == 200 && result[2] == [decoded.b] && APP.requests[0][1].b == title.b, describe(*result))
  end

  result = serve(server, request(chunked(['{"title":', '"hello"}']), "Transfer-Encoding: chunked\r\n", "application/json"), chunk: 1)
  check("#{server}: JSON params reach the action", result[0] == 200 && APP.requests[0][2] == "hello", describe(*result))
  binary = "\x00\xff\x80".b
  result = serve(server, request(chunked([binary]), "Transfer-Encoding: CHUNKED\r\n", "application/octet-stream"), chunk: 1)
  check("#{server}: binary octets are preserved", result[0] == 200 && result[2] == [binary], describe(*result))

  [
    ["unknown coding", "Transfer-Encoding: widgets\r\n", 501],
    ["unknown preceding coding", "Transfer-Encoding: widgets, chunked\r\n", 501],
    ["unsupported gzip", "Transfer-Encoding: gzip\r\n", 400],
    ["unsupported gzip chain", "Transfer-Encoding: gzip, chunked\r\n", 501],
    ["conflicting length", "Transfer-Encoding: chunked\r\nContent-Length: 1\r\n", 400],
    ["duplicate chunked", "Transfer-Encoding: chunked\r\nTransfer-Encoding: chunked\r\n", 400],
  ].each do |label, fields, expected|
    result = serve(server, request(chunked(["x"]), fields))
    check("#{server}: #{label} answers #{expected}", result[0] == expected && result[2].empty? && result[4] == false, describe(*result))
  end

  [
    ["declared over cap", "401\r\n", 413],
    ["cumulative over cap", chunked(["x" * 1024, "x"]), 413],
    ["huge hex size", "fffffffffffffffffffffffff\r\n", 413],
    ["oversized size line", "0" * 4097 + "\r\n\r\n", 400],
    ["invalid hex", "Q\r\nx\r\n0\r\n\r\n", 400],
    ["invalid data delimiter", "1\r\nx!!0\r\n\r\n", 400],
    ["invalid trailer", "0\r\nBad trailer\r\n\r\n", 400],
    ["oversized trailers", "0\r\n" + "X-T: #{'x' * 1000}\r\n" * 66 + "\r\n", 400],
  ].each do |label, encoded, expected|
    result = serve(server, request(encoded), chunk: 4096)
    check("#{server}: #{label} answers #{expected} without dispatch", result[0] == expected && result[2].empty? && result[4] == false, describe(*result))
  end
  result = serve(server, request(chunked(["x" * 1024])), chunk: 7)
  check("#{server}: the body cap itself is admitted", result[0] == 200 && result[2] == ["x" * 1024], describe(*result))

  # One connection owns the surplus from the decoder, including a successor
  # already in its current recv. Exercise each split position around the tail.
  first = request(chunked(["title=hello"], "X-T: ignored\r\n"))
  second = "GET /next HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
  [1, 7, first.bytesize - 1, first.bytesize + 3, 65535].each do |read_size|
    Sock.wire = Wire.new(first + second, chunk: read_size)
    APP.reset
    case server
    when "threaded" then Tep::Server::Threaded.handle_connection(7)
    when "scheduled" then Tep::Server::Scheduled.handle_connection(7)
    else Tep::Server.new(APP).handle_connection(7)
    end
    check("#{server}: pipelined successor, read=#{read_size}", APP.requests.map(&:first) == ["/posts", "/next"] &&
      APP.bodies == ["title=hello", ""] && Sock.wire.out.scan(/HTTP\/1\.1 200/).size == 2, APP.requests.inspect)
  end
end

# String#index must keep counting octets even when a caller tags its read
# UTF-8, including a recv split within a multibyte character.
[1, 2, 5, 64].each do |read_size|
  decoder = Tep::ChunkedBody.new(2)
  wire = "2\r\né\r\n0\r\n\r\nGET /next"
  offset = 0
  while offset < wire.bytesize
    decoder.feed(wire.byteslice(offset, read_size))
    offset += read_size
  end
  check("UTF-8-tagged decoder feed, read=#{read_size}", decoder.complete? &&
    decoder.body == "é".b && decoder.pending_input == "GET /next", decoder.status.to_s)
end
abort "chunked checks failed" unless CHECKS.all?
puts "#{CHECKS.size} checks pass"
puts "done"
