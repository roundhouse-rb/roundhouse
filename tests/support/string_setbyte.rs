//! One contract for `String#setbyte`, shared by the interpreted (CRuby
//! overlay) and native (spinel) lanes and by the strict targets' refusal.
//! The Ruby family runs it: the byte is written in place and the call
//! answers the byte written. No strict target's runtime has it, so those
//! targets refuse the call where the receiver can be a String.
//!
//! The expected lines are CRuby 4.0's output for the same code.

pub fn overlay() -> super::emit_and_run::Overlay {
    super::emit_and_run::real_blog().write("app/models/byte_probe.rb", &format!("{REGISTER}\nclass ByteProbe\n{METHODS}end\n"))
}

/// An app class with a `setbyte` of its own: not a String's.
pub const REGISTER: &str = r##"class Register
  def initialize
    @last = 0
  end

  #: (Integer, Integer) -> Integer
  def setbyte(i, b)
    @last = i + b
  end
end
"##;

/// Class methods, for a plain class here and for a model in the strict
/// targets' half: four sites on a String (`String`, `String?`, a binary
/// copy, the call sent by name) and one on `Register`.
pub const METHODS: &str = r##"  #: (String, Integer, Integer) -> Integer
  def self.poke(s, i, b)
    s.setbyte(i, b)
  end

  #: (String?, Integer, Integer) -> Integer
  def self.poke_maybe(s, i, b)
    s.setbyte(i, b)
  end

  #: (String, Integer, Integer) -> String
  def self.poke_copy(s, i, b)
    t = s.b
    t.setbyte(i, b)
    t
  end

  #: (String, Integer, Integer) -> Integer
  def self.poke_sent(s, i, b)
    s.public_send(:setbyte, i, b)
  end

  #: (Integer, Integer) -> Integer
  def self.own(i, b)
    Register.new.setbyte(i, b)
  end
"##;

/// Heap Strings (`dup`): a write in place has to reach the caller's String.
pub const SCRIPT: &str = r##"
s = "abc".dup
p ByteProbe.poke(s, 1, 90)
p s
m = "abc".dup
p ByteProbe.poke_maybe(m, 0, 122)
p m
src = "abc".dup
c = ByteProbe.poke_copy(src, 2, 33)
p c
p src
q = "abc".dup
p ByteProbe.poke_sent(q, 1, 81)
p q
p ByteProbe.own(1, 7)
"##;

pub const EXPECTED: &str = "90\n\"aZc\"\n122\n\"zbc\"\n\"ab!\"\n\"abc\"\n81\n\"aQc\"\n8\n";
