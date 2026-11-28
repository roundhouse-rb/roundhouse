# frozen_string_literal: true
# WebAuthn - passkey registration and sign-in, the relying-party half of
# the webauthn gem: `WebAuthn::Credential.options_for_create` /
# `options_for_get` answer the options a browser's `navigator.credentials`
# call takes (`as_json` is their JSON), `from_create` / `from_get` read
# what it sends back, and `verify` checks it or raises a `WebAuthn::Error`.
# This raising façade only stands in where the real implementation can't:
# the spin-shaped spinel tree swaps this file for `require "webauthn"` -
# the spinel-webauthn spin package, the gem's classes and answers -
# whenever the app names WebAuthn (see project.rs spin_shape). CRuby and
# JRuby no-op the whole façade chain - the gem is pure Ruby over openssl
# and real on both.
#
# Own file rather than a gem_facades.rb section so the swap is
# whole-file - the same grain bcrypt_facade.rb uses.
module WebAuthn
  # The errors an app rescues by name. Defined here so the names resolve
  # where the façade stands; the gem and the spin package define the same
  # classes with the same ancestry.
  class Error < StandardError
  end

  class VerificationError < Error
  end

  class ChallengeVerificationError < VerificationError
  end

  class OriginVerificationError < VerificationError
  end

  class RpIdVerificationError < VerificationError
  end

  class TypeVerificationError < VerificationError
  end

  class UserPresenceVerificationError < VerificationError
  end

  class UserVerifiedVerificationError < VerificationError
  end

  class SignatureVerificationError < VerificationError
  end

  class SignCountVerificationError < VerificationError
  end

  class AttestationStatementVerificationError < VerificationError
  end

  class Configuration
    def initialize
      @allowed_origins = []
      @rp_name = ""
      @rp_id = ""
    end

    def allowed_origins
      @allowed_origins
    end

    def allowed_origins=(origins)
      @allowed_origins = origins
    end

    def rp_name
      @rp_name
    end

    def rp_name=(name)
      @rp_name = name
    end

    def rp_id
      @rp_id
    end

    def rp_id=(id)
      @rp_id = id
    end
  end

  def self.configuration
    GemFacade.fail!("WebAuthn.configuration")
    Configuration.new
  end

  def self.configure
    GemFacade.fail!("WebAuthn.configure")
    yield Configuration.new
  end

  def self.generate_user_id
    GemFacade.fail!("WebAuthn.generate_user_id")
    ""
  end

  module PublicKeyCredential
    class CreationOptions
      def challenge
        GemFacade.fail!("WebAuthn::PublicKeyCredential::CreationOptions#challenge")
        ""
      end

      def as_json(_options = nil)
        GemFacade.fail!("WebAuthn::PublicKeyCredential::CreationOptions#as_json")
        { challenge: "" }
      end
    end

    class RequestOptions
      def challenge
        GemFacade.fail!("WebAuthn::PublicKeyCredential::RequestOptions#challenge")
        ""
      end

      def as_json(_options = nil)
        GemFacade.fail!("WebAuthn::PublicKeyCredential::RequestOptions#as_json")
        { challenge: "" }
      end
    end
  end

  class PublicKeyCredentialWithAttestation
    def id
      GemFacade.fail!("WebAuthn::PublicKeyCredentialWithAttestation#id")
      ""
    end

    def verify(_challenge, user_presence: nil, user_verification: nil)
      GemFacade.fail!("WebAuthn::PublicKeyCredentialWithAttestation#verify")
      false
    end

    def public_key
      GemFacade.fail!("WebAuthn::PublicKeyCredentialWithAttestation#public_key")
      ""
    end

    def sign_count
      GemFacade.fail!("WebAuthn::PublicKeyCredentialWithAttestation#sign_count")
      0
    end
  end

  class PublicKeyCredentialWithAssertion
    def id
      GemFacade.fail!("WebAuthn::PublicKeyCredentialWithAssertion#id")
      ""
    end

    def verify(_challenge, public_key:, sign_count:, user_presence: nil, user_verification: nil)
      GemFacade.fail!("WebAuthn::PublicKeyCredentialWithAssertion#verify")
      false
    end

    def sign_count
      GemFacade.fail!("WebAuthn::PublicKeyCredentialWithAssertion#sign_count")
      0
    end

    def user_handle
      GemFacade.fail!("WebAuthn::PublicKeyCredentialWithAssertion#user_handle")
      ""
    end
  end

  module Credential
    def self.options_for_create(user:, attestation: nil, authenticator_selection: nil, exclude_credentials: nil,
                                exclude: nil, pub_key_cred_params: nil, algs: nil, rp: nil, timeout: nil,
                                extensions: nil)
      GemFacade.fail!("WebAuthn::Credential.options_for_create")
      PublicKeyCredential::CreationOptions.new
    end

    def self.options_for_get(rp_id: nil, allow_credentials: nil, allow: nil, user_verification: nil,
                             timeout: nil, extensions: nil)
      GemFacade.fail!("WebAuthn::Credential.options_for_get")
      PublicKeyCredential::RequestOptions.new
    end

    def self.from_create(_credential)
      GemFacade.fail!("WebAuthn::Credential.from_create")
      PublicKeyCredentialWithAttestation.new
    end

    def self.from_get(_credential)
      GemFacade.fail!("WebAuthn::Credential.from_get")
      PublicKeyCredentialWithAssertion.new
    end
  end
end
