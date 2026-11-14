//! `ActiveSupport::OrderedOptions` in app code: a name no method answers reads or writes its key, as Rails'
//! `method_missing` does, on the ruby family's Hash and on Spinel's Hash-holding class alike; and a `delegate` whose
//! names are the keys of the literal Hash a class method answers (view_component's `Config.defaults.keys`); and a
//! subclass's `update`, Rails' alias of `merge!` unless the subclass defines its own.

use super::emit_and_run;

pub const SCRIPT: &str = "p SettingsBox.report\np [DelegBox.new.alpha, DelegBox.new.beta]\np PrefBox.report\n";

pub const EXPECTED: &str = "[\"app/components\", false, \"blue\", \"blue\", nil, 3, \"app/components\"]\n[1, 2]\n[3, 3, :class_side, :own]\n";

pub fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .write(
            "app/lib/pref_box.rb",
            "class PrefBox < ActiveSupport::OrderedOptions\n  def self.update\n    :class_side\n  end\n\n  def self.report\n    o = PrefBox.new\n    o.update({ width: 3 })\n    [o.width, o[:width], PrefBox.update, OwnBox.new.update({ width: 1 })]\n  end\nend\n\nclass OwnBox < ActiveSupport::OrderedOptions\n  def update(other)\n    :own\n  end\nend\n",
        )
        .write(
            "app/lib/deleg_box.rb",
            "class DelegBox\n  def self.defaults\n    ActiveSupport::OrderedOptions.new.merge!({ alpha: 1, beta: 2 })\n  end\n\n  def config\n    self.class.defaults\n  end\n\n  delegate(*DelegBox.defaults.keys, to: :config)\nend\n",
        )
        .write(
        "app/lib/settings_box.rb",
        "class SettingsBox\n  def self.options\n    options = ActiveSupport::OrderedOptions.new\n    options.merge!({ path: \"app/components\", enabled: false })\n    options.color = \"blue\"\n    options\n  end\n\n  def self.report\n    o = options\n    [o.path, o.enabled, o.color, o[:color], o.missing, o.keys.size, o.path!]\n  end\nend\n",
    )
}
