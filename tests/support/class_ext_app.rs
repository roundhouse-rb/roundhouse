//! ActiveSupport's `Class#descendants` and `module_parents` on a class, as Rails 8.1 answers them, on the ruby family
//! and on Spinel (whose runtime reopens `Class`: it answers no reopened `Module` method on a class).

use super::emit_and_run;

pub const SCRIPT: &str = "CritterReport.lines.each { |line| p line }\n";

pub const EXPECTED: &str = "3\n[\"Dog\", \"Puppy\", \"Zoo::Keeper::Cat\"]\n[\"Zoo::Keeper\", \"Zoo\", \"Object\"]\n\"Zoo::Keeper\"\n";

pub fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .write("app/lib/critter.rb", "class Critter\n  def sound = \"...\"\nend\n")
        // Not the script's: the tree loads Spinel's `Class` reopen only where its own code reads these, and carries a
        // class only where some code names it (Rails' eager load has no counterpart in a tree that is shaken).
        .write(
            "app/lib/critter_report.rb",
            "class CritterReport\n  def self.lines\n    [[Dog, Puppy, Zoo::Keeper::Cat].size, Critter.descendants.map(&:name).sort, Zoo::Keeper::Cat.module_parents.map(&:name), Zoo::Keeper::Cat.module_parent.name]\n  end\nend\n",
        )
        .write("app/lib/dog.rb", "class Dog < Critter\n  def sound = \"woof\"\nend\n")
        .write("app/lib/puppy.rb", "class Puppy < Dog\n  def sound = \"yip\"\nend\n")
        .write("app/lib/zoo/keeper/cat.rb", "module Zoo\n  module Keeper\n    class Cat < Critter\n      def sound = \"meow\"\n    end\n  end\nend\n")
}
