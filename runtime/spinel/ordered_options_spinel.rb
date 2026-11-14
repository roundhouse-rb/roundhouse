# frozen_string_literal: true

# `ActiveSupport::OrderedOptions` for the Spinel tree, after activesupport 8.1
# `lib/active_support/ordered_options.rb` (MIT license, Copyright (c) David
# Heinemeier Hansson). Rails' is a Hash whose `method_missing` reads and
# writes keys; Spinel subclasses no builtin and dispatches no
# `method_missing`, so this one holds its Hash, and a call no method answers
# is written as the `method_missing` call Ruby makes
# (`lower::method_missing_calls`). The ruby family ships Rails' own shape
# (`ruby_overlay/runtime/active_support_ordered_options.rb`).
module ActiveSupport
  class OrderedOptions
    # Hash.new's default, or its block, which is handed the options and the missing key.
    def initialize(default = nil, &block)
      @values = {}
      @default = default
      @default_proc = block
    end

    def [](key)
      name = key.to_sym
      return @values[name] if @values.key?(name)
      return @default_proc.call(self, name) unless @default_proc.nil?
      @default
    end

    def dig(key, *rest)
      value = self[key]
      return value if rest.empty? || value.nil?
      value.dig(*rest)
    end

    def []=(key, value)
      @values[key.to_sym] = value
    end

    # Hash#merge!: the keys as given, as Rails' does (only `[]=` symbolizes). Not `update` beside it: Spinel types a
    # method by every class that defines its name, and openssl's `Digest#update` is handed a String.
    def merge!(other)
      source = other.is_a?(OrderedOptions) ? other.to_h : other
      source.each { |key, value| @values[key] = value }
      self
    end

    def keys
      @values.keys
    end

    def key?(key)
      @values.key?(key)
    end

    # Hash#fetch's fallbacks: a default, or a block handed the missing key.
    def fetch(key, *default, &block)
      return @values.fetch(key) { |missing| block.call(missing) } unless block.nil?
      return @values.fetch(key, default[0]) unless default.empty?
      @values.fetch(key)
    end

    def to_h
      @values.dup
    end

    def empty?
      @values.empty?
    end

    def method_missing(name, *args)
      text = name.to_s
      if text.end_with?("=")
        self[text[0, text.length - 1].to_s] = args[0]
      elsif text.end_with?("!")
        key = text[0, text.length - 1].to_s
        value = self[key]
        # Rails' `presence`: nil, false, a whitespace-only String and an empty collection are blank.
        blank = value.nil? || value == false || (value.is_a?(String) && value.strip.empty?) || value == [] || value == {}
        raise KeyError, ":#{key} is blank" if blank
        value
      else
        self[text]
      end
    end

    def respond_to_missing?(_name, _include_private)
      true
    end
  end
end
