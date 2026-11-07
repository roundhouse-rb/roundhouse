# Count slice bytes rather than elapsed time: CI runs suites concurrently.
# CRuby can share String backing storage, whereas Spinel copies byteslice
# results. Counting their lengths catches quadratic suffix copies on either.
require_relative "../runtime/spinel/tep/chunked_body"

def decode_work(chunks)
  wire = "1\r\nx\r\n" * chunks + "0\r\n\r\n"
  sliced_bytes = 0
  trace = TracePoint.new(:c_return) do |event|
    if event.defined_class == String && event.method_id == :byteslice
      slice = event.return_value
      sliced_bytes += slice.bytesize unless slice.nil?
    end
  end
  decoder = Tep::ChunkedBody.new(chunks)
  trace.enable { decoder.feed(wire) }
  unless decoder.complete? && decoder.body == "x" * chunks
    raise "incorrect work fixture"
  end
  raise "no byte-slice work measured" if sliced_bytes == 0
  sliced_bytes
end

small = decode_work(1365)
large = decode_work(10920)
puts "1365 chunks: #{small} sliced bytes"
puts "10920 chunks: #{large} sliced bytes"
raise "chunk decoding copies superlinear bytes" if large > small * 16
puts "done"
