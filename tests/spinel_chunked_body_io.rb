require_relative "tep_server_harness"

class << IO
  attr_accessor :wrapper_error, :wrapper_calls
  alias for_fd_without_failure for_fd
  def for_fd(fd, autoclose: false)
    self.wrapper_calls += 1
    raise wrapper_error unless wrapper_error.nil?
    for_fd_without_failure(fd, autoclose: autoclose)
  end
end

module Sock
  class << self
    attr_accessor :closed_clients
  end
  def self.sp_net_close(fd)
    closed_clients << fd
    0
  end
end

first = "POST /posts HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n" \
  "Content-Type: application/x-www-form-urlencoded\r\nConnection: close\r\n\r\n" \
  "b\r\ntitle=hello\r\n0\r\n\r\n"
second = "GET /next HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"

[Errno::EMFILE.new, IOError.new("wrapper setup failed")].each do |error|
  %w[blocking scheduled].each do |mode|
    IO.wrapper_error = error
    IO.wrapper_calls = 0
    Sock.closed_clients = []
    APP.reset
    handler = mode == "blocking" ? Tep::Server.new(APP) : Tep::Server::Scheduled
    raised = nil
    begin
      [first, second].each_with_index do |bytes, index|
        Sock.wire = Wire.new(bytes, chunk: 1)
        handler.handle_connection(7 + index)
      end
    rescue StandardError => caught
      raised = caught
    ensure
      IO.wrapper_error = nil
    end
    label = "#{mode}, #{error.class}"
    check("#{label}: wrapper failure does not escape the connection loop", raised.nil?, raised.inspect)
    check("#{label}: both clients close and the next request serves",
      Sock.closed_clients == [7, 8] && APP.bodies.last == "", Sock.closed_clients.inspect)
    if mode == "scheduled"
      check("#{label}: no wrapper is created and the chunked body dispatches",
        IO.wrapper_calls == 0 && APP.bodies == ["title=hello", ""], APP.bodies.inspect)
    else
      check("#{label}: failed wrapper setup drops the body without dispatch",
        IO.wrapper_calls == 1 && APP.bodies == [""], APP.bodies.inspect)
    end
  end
end
abort "chunked wrapper checks failed" unless CHECKS.all?
puts "done"
