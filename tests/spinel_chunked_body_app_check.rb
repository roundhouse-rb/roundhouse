require "socket"
require "tempfile"
require "timeout"
require_relative "support/server_check_io"

# The emitted scaffold exposes its threaded and fiber server selections.
%w[thread fiber].each do |mode|
  port = TCPServer.open("127.0.0.1", 0) { |socket| socket.addr[1] }
  Tempfile.create("chunked-action") do |log|
    pid = Process.spawn({"PORT" => port.to_s, "TEP_SERVER" => mode,
      "SPINEL_WORKERS" => "1", "BLOG_DB" => "chunked-#{mode}.sqlite3",
      "SECRET_KEY_BASE" => "synthetic-chunk-key" * 4}, ARGV.fetch(0), out: log, err: log)
    begin
      Timeout.timeout(20) do
        loop do
          begin
            TCPSocket.open("127.0.0.1", port, &:close)
            break
          rescue Errno::ECONNREFUSED
            sleep 0.01
          end
        end
      end
      [
        ["application/x-www-form-urlencoded", "6;name=one\r\ntitle=\r\n5\r\nhello\r\n0\r\nX-T: ignored\r\n\r\n", 11],
        ["application/json", "9\r\n{\"title\":\r\n8\r\n\"hello\"}\r\n0\r\n\r\n", 17],
      ].each do |content_type, body, length|
        response = TCPSocket.open("127.0.0.1", port) do |socket|
          Timeout.timeout(10) do
            socket.write("POST /frames HTTP/1.1\r\nHost: localhost\r\nContent-Type: #{content_type}\r\n" \
              "Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n" + body)
          end
          ServerCheckIO.read_to_close(socket)
        end
        raise "#{mode}: controller params/env differ: #{response.inspect}" unless response.start_with?("HTTP/1.1 200") && response.end_with?("hello|#{length}|")
        puts "ok #{mode}: generated controller reads #{content_type} params and normalized headers"
      end
    ensure
      Process.kill("TERM", pid)
      Process.wait(pid)
    end
  end
end
