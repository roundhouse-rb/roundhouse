# frozen_string_literal: true

# A String that is html_safe, for a Spinel tree whose code passes markup across values the emit cannot see (a
# ViewComponent's slots: a template block's markup, or a Ruby block's text the component's `capture` escapes). The
# ruby family's overlay has the same shape (`ruby_overlay/runtime/active_support_core_ext.rb`,
# `action_view_safe_buffer.rb`); Spinel compiles a String subclass and a reopened String as Ruby does. Loaded only by a
# tree that renders components, after the shared view helpers it overrides (`inject_spinel_active_support_requires`).
class SafeString < String
  def html_safe?
    true
  end

  def html_safe
    self
  end

  # String#to_s on a subclass answers a plain copy; the mark has to survive the `.to_s` a slot's content takes.
  def to_s
    self
  end

  # Rails' `SafeBuffer#+`: still safe, the other side escaped unless it is safe too.
  def +(other)
    SafeString.new(String.new(self) + ActionView::ViewHelpers.html_escape(other))
  end

  # Rails' `SafeBuffer#<<` / `concat`: appended escaped unless it is safe, so the buffer stays what it says.
  def <<(other)
    super(ActionView::ViewHelpers.html_escape(other))
  end

  def concat(other)
    super(ActionView::ViewHelpers.html_escape(other))
  end

  # Rails' other writers (`replace`, `insert`, `prepend`, `[]=`): what they store escaped unless it is safe.
  def replace(other)
    super(ActionView::ViewHelpers.html_escape(other))
  end

  def insert(index, other)
    super(index, ActionView::ViewHelpers.html_escape(other))
  end

  def prepend(other)
    super(ActionView::ViewHelpers.html_escape(other))
  end

  def []=(index, other)
    super(index, ActionView::ViewHelpers.html_escape(other))
  end

  # Rails' `safe_concat`: appended as it is (a safe String passes the escaping `replace` untouched).
  def safe_concat(other)
    replace(SafeString.new(String.new(self) + other.to_s))
  end
end

class String
  def html_safe
    SafeString.new(self)
  end

  def html_safe?
    false
  end
end

module ActionView
  module ViewHelpers
    # Rails' `ERB::Util.html_escape`: `to_s` first, a safe String as it is.
    def self.html_escape(s)
      text = s.to_s
      return text if text.html_safe?
      # A safe String, as Rails' is: escaping it again (a `capture` of an escaped value) leaves it as it is.
      return SafeString.new(text) unless text.include?("&") || text.include?("<") || text.include?(">") || text.include?("\"") || text.include?("'")
      SafeString.new(text.gsub(HTML_ESCAPE_PATTERN, HTML_ESCAPES))
    end

    def self.raw(value)
      SafeString.new(value.to_s)
    end
  end

  class Base
    # Rails' `capture`: the block's value, a String escaped unless it is html_safe.
    def capture(*args)
      value = yield(*args)
      # Rails 7.2 takes an OutputBuffer's text as it is (already safe).
      return SafeString.new(value.to_s) if value.is_a?(ActionView::OutputBuffer)
      return nil unless value.is_a?(String)
      ActionView::ViewHelpers.html_escape(value)
    end
  end
end
