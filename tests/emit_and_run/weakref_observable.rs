//! `WeakRef` and `include Observable`, two stdlib names an app reaches
//! through `require "weakref"` and `require "observer"` (a cache slot that
//! lets its value go, a model that notifies watchers). Both are Ruby's own
//! on the ruby family; spinel has `packages/weakref`, and its observer
//! package does not build in a Rails-sized tree yet, so `Observable` is
//! reported there as it is on the strict targets.

use super::emit_and_run;

const SLOT: &str = r#"require "weakref"

class CacheSlot
  def initialize(value)
    @ref = WeakRef.new(value)
  end

  def alive? = @ref.weakref_alive?

  def value = @ref.__getobj__
end
"#;

const TICKER: &str = r#"require "observer"

class Ticker
  include Observable

  def tick(price)
    changed
    notify_observers(price)
    count_observers
  end
end

class Watcher
  attr_reader :seen

  def initialize = @seen = []

  def update(price) = @seen << price
end
"#;

fn slot_app() -> emit_and_run::Overlay {
    emit_and_run::real_blog().write("app/models/cache_slot.rb", SLOT)
}

fn ticker_app() -> emit_and_run::Overlay {
    emit_and_run::real_blog().write("app/models/ticker.rb", TICKER)
}

const SLOT_ASSERTIONS: &str = r##"value = +"kept"
slot = CacheSlot.new(value)
raise slot.alive?.inspect unless slot.alive? == true
raise slot.value.inspect unless slot.value == "kept"
puts "weakref passed"
"##;

const TICKER_ASSERTIONS: &str = r##"ticker = Ticker.new
watcher = Watcher.new
ticker.add_observer(watcher)
got = [ticker.tick(10), ticker.tick(11), watcher.seen]
raise got.inspect unless got == [1, 1, [10, 11]]
puts "observable passed"
"##;

#[test]
fn weakref_runs() {
    let run = slot_app().run_ruby(SLOT_ASSERTIONS);
    run.assert_passes();
    assert!(run.stdout.contains("weakref passed"), "{}", run.stdout);
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn weakref_runs_on_spinel() {
    let run = slot_app().run_spinel(SLOT_ASSERTIONS);
    run.assert_passes();
    assert!(run.stdout.contains("weakref passed"), "{}", run.stdout);
}

#[test]
fn observable_runs() {
    let run = ticker_app().run_ruby(TICKER_ASSERTIONS);
    run.assert_passes();
    assert!(run.stdout.contains("observable passed"), "{}", run.stdout);
}

/// No strict runtime carries either library, and spinel's boot.rb leaves
/// `observer` out (`SPINEL_LACKS`), so those emits report the name.
#[test]
fn weakref_and_observable_are_reported_where_absent() {
    use roundhouse::project::BuildTarget::*;
    for target in [Rust, Typescript, Go, Python, CSharp, Kotlin, Swift, Elixir, Crystal] {
        let (_emitted, errors) = slot_app().emit(target);
        assert!(errors.iter().any(|e| e.contains("WeakRef")), "{target:?}: {errors:#?}");
    }
    for target in [Spinel, Rust, Typescript, Go, Python, CSharp, Kotlin, Swift, Elixir, Crystal] {
        let (_emitted, errors) = ticker_app().emit(target);
        assert!(errors.iter().any(|e| e.contains("Observable")), "{target:?}: {errors:#?}");
    }
}
