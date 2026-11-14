# frozen_string_literal: true
require "minitest/autorun"
require_relative "../test_helper"

class AuthenticityTokenTest < Minitest::Test
  def setup
    ActionController::Current.controller = ActionController::Base.new
  end

  def teardown
    ActionController::Current.controller = nil
    ActionController::Current.request = nil
  end

  def test_masked_token_differs_from_the_session_secret_and_verifies
    first = ActionController::AuthenticityToken.masked
    second = ActionController::AuthenticityToken.masked
    secret = ActionController::Current.session[:_csrf_token].to_s
    refute_equal "", first
    refute_equal "", second
    refute_equal first, second
    refute_equal first, secret
    assert ActionController::AuthenticityToken.valid?(first, secret)
    assert ActionController::AuthenticityToken.valid?(second, secret)
    assert ActionController::AuthenticityToken.valid?(secret, secret)
  end

  def test_a_foreign_or_empty_token_is_refused
    ActionController::AuthenticityToken.masked
    secret = ActionController::Current.session[:_csrf_token].to_s
    refute ActionController::AuthenticityToken.valid?("", secret)
    refute ActionController::AuthenticityToken.valid?(secret.reverse, secret)
    refute ActionController::AuthenticityToken.valid?("!!!!", secret)
    refute ActionController::AuthenticityToken.valid?(secret, "")
  end

  # The XOR on bytes a random pad can hold: all zeros against all ones, and
  # a NUL at the first, a middle and the last byte beside high bytes (a copy
  # sized up to the first NUL would lose the tail). The pad comes back as it
  # was.
  def test_xor_of_fixed_vectors
    zero = ("\0" * 32).b
    full = ("\xFF" * 32).b
    assert_equal full.bytes, ActionController::AuthenticityToken.xor(zero, full).bytes
    secret = ("\x0F" * 32).b
    [
      ("\0" + ("\x80" * 31)).b,
      (("\x80" * 15) + "\0" + ("\x7F" * 16)).b,
      (("\xFF" * 31) + "\0").b,
    ].each do |pad|
      before = pad.bytes
      got = ActionController::AuthenticityToken.xor(pad, secret)
      assert_equal before.map { |x| x ^ 0x0F }, got.bytes
      assert_equal 32, got.bytesize
      assert_equal before, pad.bytes
    end
  end

  def test_view_helpers_read_the_masked_token
    token = ActionView::ViewHelpers.form_authenticity_token
    secret = ActionController::Current.session[:_csrf_token].to_s
    assert ActionController::AuthenticityToken.valid?(token, secret)
    tags = ActionView::ViewHelpers.csrf_meta_tags
    assert_includes tags, %(name="csrf-token")
    hidden = ActionView::ViewHelpers.csrf_token_hidden_input
    assert_includes hidden, %(name="authenticity_token")
    hidden_token = hidden[/value="([^"]+)"/, 1]
    refute_equal secret, hidden_token
    assert ActionController::AuthenticityToken.valid?(hidden_token, secret)
  end
end
