//! A configuration DSL in doorkeeper 5.9's shape: a module the config
//! class `extend`s, whose `option` defines the config's reader with
//! `define_method` and, inside `builder_class.instance_eval`, the
//! builder's writer, which sets the config's ivar by a computed name.
//! The expansion writes both as ordinary `def`s.

pub const OPTION: &str = r##"module Settings
  class Config
    module Option
      def option(name, options = {})
        attribute = options[:as] || name

        builder_class.instance_eval do
          if method_defined?(name)
            Kernel.warn "#{name} already defined"
            remove_method name
          end

          define_method name do |*args, &block|
            value = block || args.first
            @config.instance_variable_set(:"@#{attribute}", value)
          end
        end

        define_method attribute do |*_args|
          if instance_variable_defined?(:"@#{attribute}")
            instance_variable_get(:"@#{attribute}")
          else
            options[:default]
          end
        end

        public attribute
      end
    end
  end
end
"##;

/// `config.rb` with the option declarations given.
pub fn config(options: &str, extra: &str) -> String {
    format!(
        r##"module Settings
  class Config
    class Builder
      attr_reader :config

      def initialize(config = Config.new, &block)
        @config = config
        instance_eval(&block) if block_given?
      end

      def build
        @config
      end
    end

    mattr_reader(:builder_class) {{ Builder }}

    extend Option

{options}
{extra}
  end

  def self.configure(&block)
    @config = Config::Builder.new(&block).build
  end

  def self.config
    @config || configure
  end
end
"##
    )
}

pub const LITERAL_OPTIONS: &str = r##"    option :base_controller, default: "ActionController::Base"
    option :api_only, default: false
    option :realm, as: :realm_name, default: "Roundhouse""##;

pub const LITERAL_ASSERTIONS: &str = r##"plain = Settings::Config::Builder.new.build
raise "default: #{plain.base_controller.inspect}" unless plain.base_controller == "ActionController::Base"
raise "as: #{plain.realm_name.inspect}" unless plain.realm_name == "Roundhouse"
raise "false default: #{plain.api_only.inspect}" unless plain.api_only == false
api = Settings.configure { api_only true; realm "Mine" }
raise "written: #{api.api_only.inspect} #{api.realm_name.inspect}" unless api.api_only == true && api.realm_name == "Mine"
own = Settings.configure { base_controller "ApplicationController" }
raise "own: #{own.base_controller.inspect}" unless own.base_controller == "ApplicationController"
# A nil written is still written: the default no longer answers.
none = Settings.configure { realm nil }
raise "nil written: #{none.realm_name.inspect}" unless none.realm_name.nil?
puts "option DSL passed"
"##;

/// doorkeeper's `resolve_controller`: a lambda default run with `instance_exec`, so it reads the config's own options.
pub const RESOLVING_OPTIONS: &str = r##"    option :base_controller, default: (lambda do
      api_only ? "ActionController::API" : "ActionController::Base"
    end)
    option :api_only, default: false"##;

pub const RESOLVE: &str = r##"    def resolve_controller(name)
      value = public_send(:"#{name}_controller")
      value.respond_to?(:call) ? instance_exec(&value) : value
    end"##;

/// doorkeeper 5.9's own: the controller the option names, as a class.
pub const RESOLVE_CLASS: &str = r##"    def resolve_controller(name)
      config_option = public_send(:"#{name}_controller")
      controller_name = if config_option.respond_to?(:call)
                          instance_exec(&config_option)
                        else
                          config_option
                        end

      controller_name.constantize
    end"##;
