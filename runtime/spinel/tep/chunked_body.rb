# frozen_string_literal: true

module Tep
  # Incremental chunk framing. Advance an offset within each read; compact
  # only before the next read, so many small chunks never copy the suffix.
  class ChunkedBody
    LINE_BYTES = 4096
    EXTENSION_BYTES = 16384
    TRAILER_BYTES = 65535

    def initialize(maximum)
      @chunk_limit = maximum
      @chunk_buffer = +"".b
      @chunk_offset = 0
      @chunk_left = 0
      @chunk_state = 0
      @chunk_extensions = 0
      @chunk_trailers = 0
      @chunk_body = +"".b
      @chunk_status = 0
    end

    def body
      @chunk_body
    end

    def status
      @chunk_status
    end

    def complete?
      @chunk_state == 4 && @chunk_status == 0
    end

    def finished?
      @chunk_state == 4 || @chunk_status != 0
    end

    def pending_input
      @chunk_buffer.byteslice(@chunk_offset, @chunk_buffer.bytesize - @chunk_offset)
    end

    def feed(bytes)
      @chunk_buffer = pending_input if @chunk_offset > 0
      @chunk_offset = 0
      @chunk_buffer << bytes.b
      until finished?
        available = @chunk_buffer.bytesize - @chunk_offset
        if @chunk_state == 0
          line = take_line
          return nil if line.nil? || @chunk_status != 0
          size = read_size(line)
          return nil if @chunk_status != 0
          if size > @chunk_limit - @chunk_body.bytesize
            @chunk_status = 413
            return nil
          end
          @chunk_left = size
          @chunk_state = size == 0 ? 3 : 1
        elsif @chunk_state == 1
          return nil if available == 0
          count = [available, @chunk_left].min
          @chunk_body << @chunk_buffer.byteslice(@chunk_offset, count)
          @chunk_offset += count
          @chunk_left -= count
          @chunk_state = 2 if @chunk_left == 0
        elsif @chunk_state == 2
          return nil if available < 2
          if @chunk_buffer.byteslice(@chunk_offset, 2) != "\r\n"
            @chunk_status = 400
            return nil
          end
          @chunk_offset += 2
          @chunk_state = 0
        else
          line = take_line
          return nil if line.nil? || @chunk_status != 0
          @chunk_trailers += line.bytesize + 2
          if @chunk_trailers > TRAILER_BYTES || (!line.empty? && !valid_trailer?(line))
            @chunk_status = 400
            return nil
          end
          @chunk_state = 4 if line.empty?
        end
      end
      nil
    end

    def take_line
      ending = @chunk_buffer.index("\r\n", @chunk_offset)
      available = @chunk_buffer.bytesize - @chunk_offset
      if ending.nil?
        if available > LINE_BYTES + 1 || (available > LINE_BYTES && !@chunk_buffer.end_with?("\r"))
          @chunk_status = 400
        end
        return nil
      end
      length = ending - @chunk_offset
      if length > LINE_BYTES
        @chunk_status = 400
        return nil
      end
      line = @chunk_buffer.byteslice(@chunk_offset, length)
      @chunk_offset = ending + 2
      line
    end

    def read_size(line)
      ending = line.index(";")
      ending = line.bytesize if ending.nil?
      digits = line.byteslice(0, ending)
      if !digits.match?(/\A[0-9a-fA-F]+\z/) || line.match?(/[\x00-\x08\x0a-\x1f\x7f]/)
        @chunk_status = 400
        return 0
      end
      @chunk_extensions += line.bytesize - ending
      if @chunk_extensions > EXTENSION_BYTES
        @chunk_status = 400
        return 0
      end
      size = 0
      digits.each_byte do |byte|
        digit = byte <= 57 ? byte - 48 : (byte <= 70 ? byte - 55 : byte - 87)
        # Saturation avoids overflow before the cap comparison on Spinel.
        size = size > @chunk_limit / 16 ? @chunk_limit + 1 : size * 16 + digit
      end
      size
    end

    def valid_trailer?(line)
      line.match?(/\A[!#$%&'*+.^_`|~0-9A-Za-z-]+:[^\x00-\x08\x0a-\x1f\x7f]*\z/)
    end
  end
end
