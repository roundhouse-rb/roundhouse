require_relative "../runtime/spinel/tep/tep"

module Db
  def self.with_connection
    yield
    true
  end
  def self.read_snapshot_begin; end
  def self.read_snapshot_end; end
end

module Main
  def self.dispatch(req, res)
    if req.path == "/posts"
      res.body = req.req_params["title"] + "|" + req.req_headers["content-length"] +
        "|" + req.req_headers.key?("transfer-encoding").to_s
    else
      res.body = "alive"
    end
    nil
  end
end

Tep::APP.name = "synthetic"
Tep::APP.bind_host = "127.0.0.1"
port = ARGV[0].to_i
mode = ARGV[1]
if mode == "threaded"
  Tep::Server::Threaded.new(Tep::APP).run(port, 1, false)
elsif mode == "scheduled"
  Tep::Server::Scheduled.new(Tep::APP).run(port, 1, false)
else
  Tep::Server.new(Tep::APP).run(port, 1, false)
end
