# Local benchmark: ruby scripts/bench-chunked-body.rb on a quiet host.
# No test harness runs this: concurrent CI suites make elapsed-time ratios noisy.
require_relative "../runtime/spinel/tep/chunked_body"

def decode_time(chunks)
  wire = "1\r\nx\r\n" * chunks + "0\r\n\r\n"
  samples = []
  9.times do
    started = Process.clock_gettime(Process::CLOCK_MONOTONIC)
    20.times do
      decoder = Tep::ChunkedBody.new(chunks)
      decoder.feed(wire)
      raise "incorrect timing fixture" unless decoder.complete? && decoder.body.bytesize == chunks
    end
    samples << (Process.clock_gettime(Process::CLOCK_MONOTONIC) - started) / 20
  end
  samples.sort[4]
end

small = decode_time(1365)
large = decode_time(10920)
ratio = large / small
puts "8195 encoded bytes (1365 chunks): #{(small * 1000).round(3)} ms"
puts "65525 encoded bytes (10920 chunks): #{(large * 1000).round(3)} ms"
puts "8x chunks: #{ratio.round(3)}x median elapsed time"
raise "chunk decoding grows superlinearly" if ratio > 16
puts "done"
