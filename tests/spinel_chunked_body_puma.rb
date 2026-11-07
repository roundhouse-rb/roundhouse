require_relative "support/rails_oracle"
require_relative "support/server_check_io"
unless RailsOracle.available?("railties", "actionpack", puma: "~> 8.0")
  puts "skip: #{RailsOracle.label} and Puma 8 are required"
  exit
end
RailsOracle.activate("railties", "actionpack", puma: "~> 8.0")
require "rails"
require "action_controller/railtie"
require "puma"
require "socket"
require "stringio"

class ChunkOracle < Rails::Application
  config.load_defaults 8.1
  config.eager_load = false
  config.secret_key_base = "synthetic-chunk-oracle"
  config.hosts.clear
  config.action_controller.allow_forgery_protection = false
  config.logger = Logger.new(File::NULL)
end
class ChunksController < ActionController::Base
  skip_forgery_protection
  def echo
    render plain: [params[:title], request.content_length,
      request.env.key?("HTTP_TRANSFER_ENCODING"), request.env.key?("HTTP_X_TRACE")].join("|")
  end
  def alive
    render plain: "alive"
  end
end
ChunkOracle.initialize!
ChunkOracle.routes.draw do
  post "/posts", to: "chunks#echo"
  get "/next", to: "chunks#alive"
end
log = StringIO.new
server = Puma::Server.new(ChunkOracle, Puma::Events.new, min_threads: 0, max_threads: 1,
  http_content_length_limit: 1024, log_writer: Puma::LogWriter.new(log, log))
listener = server.add_tcp_listener("127.0.0.1", 0)
port = listener.addr[1]
server.run
def request(body, coding = "chunked", close = true, content_type = "application/x-www-form-urlencoded")
  "POST /posts HTTP/1.1\r\nHost: localhost\r\nContent-Type: #{content_type}\r\nTransfer-Encoding: #{coding}\r\n#{close ? "Connection: close\r\n" : ""}\r\n".b + body.b
end
def exchange(port, bytes, split = false)
  TCPSocket.open("127.0.0.1", port) do |socket|
    if split
      bytes.each_byte { |byte| socket.write(byte.chr); sleep 0.0001 }
    else
      socket.write(bytes)
    end
    ServerCheckIO.read_to_close(socket, timeout: 5)
  end
end
begin
  body = "6;name=one\r\ntitle=\r\n5;name=\"a;b\"\r\nhello\r\n0\r\nX-Trace: ignored\r\n\r\n"
  [false, true].each do |split|
    # Puma 8.0's trailer parser requires the trailer block in its read.
    # Confirm split chunk delimiters separately, without trailers.
    payload = split ? body.sub("X-Trace: ignored\r\n", "") : body
    response = exchange(port, request(payload), split)
    raise "decoded action/env differs: #{response.inspect}" unless response.start_with?("HTTP/1.1 200") && response.end_with?("hello|11|false|false")
    puts "ok Puma: decoded params, normalized env, extensions/trailers, split=#{split}"
  end
  response = exchange(port, request("9\r\n{\"title\":\r\n8\r\n\"hello\"}\r\n0\r\n\r\n", "chunked", true, "application/json"))
  raise "JSON action/env differs" unless response.start_with?("HTTP/1.1 200") && response.end_with?("hello|17|false|false")
  puts "ok Puma: decoded JSON params and normalized env"
  {
    "widgets" => 501, "widgets, chunked" => 501,
    "gzip" => 400, "identity" => 501, "chunked, gzip" => 400,
  }.each do |coding, expected|
    response = exchange(port, request("0\r\n\r\n", coding))
    raise "#{coding}: expected #{expected}, got #{response[0,30]}" unless response.start_with?("HTTP/1.1 #{expected}")
    puts "ok Puma: #{coding} => #{expected}"
  end
  response = exchange(port, request("401\r\n" + "x" * 1025 + "\r\n0\r\n\r\n"))
  raise "body cap differs" unless response.start_with?("HTTP/1.1 413")
  puts "ok Puma: over-cap body => 413"
  response = exchange(port, request("0" * 4097))
  raise "chunk-size line cap differs" unless response.start_with?("HTTP/1.1 400")
  puts "ok Puma: oversized chunk-size line => 400"
  response = exchange(port, request(body, "chunked", false) +
    "GET /next HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
  raise "pipeline differs: #{response.inspect}" unless response.scan(/HTTP\/1\.1 200/).size == 2 && response.end_with?("alive")
  puts "ok Puma: pipelined second request"
  puts "Puma #{Puma::Const::PUMA_VERSION}, Rails #{Rails.version}, Ruby #{RUBY_VERSION}, load_defaults=8.1, body cap=1024: done"
ensure
  server.stop(true)
end
