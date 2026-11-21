//! multi_json on its oj adapter (`ingest::multi_json`): `MultiJson.dump` as oj's compat dump.

use super::emit_and_run;

pub(super) const PAYLOAD: &str = r##"class Payload
  def self.flat
    MultiJson.dump({ "a" => 1, :b => [1, 2, 3], :s => "<a>&' é\"\\\n/" })
  end

  def self.floats
    MultiJson.dump([1.0, 1e20, 1.0e-5, 0.1 + 0.2, 100.0, 1e17, 123456789.123456789, -0.0, 2.5e-7, 9.2e18, 9.3e18, 1e300, 1.0 / 3, 5e-324, 0.10000000000000009, 1.0000000000000002, 0.19999999999999998, 2.0000000000000005e-05, 9.9206327263])
  end

  def self.special
    MultiJson.dump([Float::NAN, Float::INFINITY, -Float::INFINITY])
  end

  def self.time
    MultiJson.dump({ at: Time.utc(2024, 1, 2, 3, 4, 5) })
  end

  def self.pretty
    MultiJson.dump({ a: [1, 2], e: {}, f: [], g: { h: "i" } }, pretty: true)
  end

  def self.pretty_false
    MultiJson.dump([1, [2]], pretty: false)
  end

  def self.symbols
    MultiJson.dump({ a: { b: :c }, l: [:x, :y] })
  end

  def self.scalars
    [MultiJson.dump(nil), MultiJson.dump(true), MultiJson.dump("x"), MultiJson.dump(:sym), MultiJson.dump(12), MultiJson.dump(2.5)].join(" ")
  end

  def self.positional
    MultiJson.dump([1], { pretty: true })
  end

  def self.kwargs
    MultiJson.dump(html_content: "<p>", text_content: "t")
  end

  def self.rescued
    MultiJson.dump([1])
  rescue MultiJson::LoadError, MultiJson::DecodeError
    "rescued"
  end
end
"##;

/// Each line is what multi_json 1.21.1 over oj 3.17.6 (compat, ActiveSupport and `Oj::Rails.optimize` loaded, as
/// discourse's initializer sets them up) answers for `Payload`'s method of that name.
pub(super) const ASSERTIONS: &str = r##"expected = {
  flat: "{\"a\":1,\"b\":[1,2,3],\"s\":\"<a>&'\u2028é\\\"\\\\\\n/\"}",
  floats: "[1.0,1e+20,1e-05,0.3,100.0,100000000000000000.0,123456789.1234568,0.0,2.5e-07,9200000000000000000.0,9.3e+18,1e+300,0.3333333333333333,4.940656458412465e-324,0.10000000000000009,1,0.2,2.000000000000001e-05,9.9206327263]",
  special: "[NaN,Infinity,-Infinity]",
  time: "{\"at\":\"2024-01-02T03:04:05.000Z\"}",
  pretty: "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"e\": {},\n  \"f\": [],\n  \"g\": {\n    \"h\": \"i\"\n  }\n}",
  pretty_false: "[\n  1,\n  [\n    2\n  ]\n]",
  symbols: "{\"a\":{\"b\":\"c\"},\"l\":[\"x\",\"y\"]}",
  scalars: "null true \"x\" \"sym\" 12 2.5",
  positional: "[\n  1\n]",
  kwargs: "{\"html_content\":\"<p>\",\"text_content\":\"t\"}",
}
actual = {
  flat: Payload.flat, floats: Payload.floats, special: Payload.special, time: Payload.time, pretty: Payload.pretty,
  pretty_false: Payload.pretty_false, positional: Payload.positional, symbols: Payload.symbols, scalars: Payload.scalars, kwargs: Payload.kwargs,
}
raise "rescued" unless Payload.rescued == "[1]"
raise "aliases" unless MultiJson::ParseError.new("x").is_a?(MultiJson::LoadError) && MultiJson::ParseError.new("x").is_a?(MultiJson::DecodeError)
expected.each { |name, want| raise "#{name}: #{actual[name].inspect}" unless actual[name] == want }
puts "multi_json passed"
"##;

pub(super) const INITIALIZER: &str = "# frozen_string_literal: true\n\nOj::Rails.set_encoder()\nOj::Rails.set_decoder()\nOj::Rails.optimize()\nOj.default_options = Oj.default_options.merge(mode: :compat)\nMultiJSON.use(:oj)\n";

pub(super) fn overlay(overlay: emit_and_run::Overlay) -> emit_and_run::Overlay {
    overlay
        .edit("Gemfile.lock", "  specs:\n", "  specs:\n    multi_json (1.21.1)\n    oj (3.17.6)\n")
        .write("config/initializers/100-oj.rb", INITIALIZER)
        .write("lib/payload.rb", PAYLOAD)
}

#[test]
fn multi_json_dump_runs_as_ojs_compat_dump() {
    let run = overlay(emit_and_run::real_blog()).run_ruby(ASSERTIONS);
    run.assert_passes();
    assert!(run.stdout.contains("multi_json passed"));
}

#[test]
fn multi_json_dump_runs_as_ojs_compat_dump_on_spinel() {
    let run = overlay(emit_and_run::real_blog()).run_spinel(ASSERTIONS);
    run.assert_passes();
    assert!(run.stdout.contains("multi_json passed"));
}

/// A value oj would write through its `to_json` (a record, an object, an untyped value) is refused: Rails' encoder
/// writes it, which the stand-in does not model.
#[test]
fn multi_json_dump_of_an_object_is_refused() {
    let lib = "class Dumped\n  def self.record\n    MultiJson.dump(Article.first)\n  end\n\n  def self.mixed\n    MultiJson.dump({ a: Article.first })\n  end\n\n  def self.through_lambda\n    encode = ->(value) { MultiJson.dump(value) }\n    encode.call(1)\n  end\nend\n";
    let (_emitted, errors) = overlay(emit_and_run::real_blog())
        .write("lib/dumped.rb", lib)
        .emit(roundhouse::project::BuildTarget::Ruby);
    let refused: Vec<&String> = errors.iter().filter(|e| e.contains("MultiJson.dump")).collect();
    assert_eq!(refused.len(), 2, "{errors:#?}");
    assert!(refused.iter().all(|e| e.contains("Article")), "{refused:#?}");
}

/// Without an initializer that picks oj, multi_json 1.21 dumps with json_gem, which is not modeled: `MultiJson`
/// stays an unsupported constant.
#[test]
fn multi_json_without_the_oj_adapter_stays_unsupported() {
    let (_emitted, errors) = emit_and_run::real_blog()
        .edit("Gemfile.lock", "  specs:\n", "  specs:\n    multi_json (1.21.1)\n    oj (3.17.6)\n")
        .write("lib/payload.rb", PAYLOAD)
        .emit(roundhouse::project::BuildTarget::Ruby);
    assert!(errors.iter().any(|e| e.contains("MultiJson")), "{errors:#?}");
}

/// A target that does not emit the stand-in refuses the call in one place.
#[test]
fn multi_json_stand_in_is_refused_off_the_ruby_family() {
    let (_emitted, errors) = overlay(emit_and_run::real_blog()).emit(roundhouse::project::BuildTarget::Python);
    assert!(errors.iter().any(|e| e.contains("MultiJson") && e.contains("stand-in")), "{errors:#?}");
}

const LOCK: &str = "GEM\n  remote: https://rubygems.org/\n  specs:\n    multi_json (1.21.1)\n    oj (3.17.6)\n\nPLATFORMS\n  ruby\n\nDEPENDENCIES\n  multi_json\n  oj\n";

fn tree(lock: &str, init: &str, lib: &str) -> std::collections::HashMap<std::path::PathBuf, Vec<u8>> {
    [
        ("Gemfile.lock", lock),
        ("config/initializers/multi_json.rb", init),
        ("lib/codec.rb", lib),
        ("config/routes.rb", "Rails.application.routes.draw do\nend\n"),
    ]
    .into_iter()
    .map(|(p, s)| (std::path::PathBuf::from(p), s.as_bytes().to_vec()))
    .collect()
}

/// Each configuration this does not model is a refusal (strict ingest fails, survey mode ledgers it) and installs no
/// stand-in; `MultiJson.load` is a refusal of its own beside a lowered `dump`.
#[test]
fn multi_json_unmodeled_configurations_are_refused() {
    let dump = "class Codec\n  def self.out(v)\n    MultiJson.dump(v)\n  end\nend\n";
    let old_lock = LOCK.replace("multi_json (1.21.1)", "multi_json (1.15.0)");
    for (lock, init, lib, expected, installed) in [
        (LOCK, "MultiJson.use(:oj)\n", "class Codec\n  def self.in(v)\n    MultiJson.load(v)\n  end\n\n  def self.out(v)\n    MultiJson.dump(v)\n  end\nend\n", "`MultiJson.load`: oj's strict parser", true),
        (LOCK, "MultiJson.use(:oj)\n", "class Codec\n  def self.out(v)\n    MultiJson.dump(v, indent: 4)\n  end\nend\n", "`MultiJson.dump(v, indent: 4)`", true),
        (LOCK, "MultiJson.use(:json_gem)\n", dump, "only the oj adapter is modeled", false),
        (LOCK, "MultiJson.use(:oj) if ENV[\"OJ\"]\n", dump, "not chosen at an initializer's top level", false),
        (LOCK, "Oj.default_options = Oj.default_options.merge(mode: :compat, float_precision: 4)\nMultiJson.use(:oj)\n", dump, "sets more than the mode", false),
        (LOCK, "MultiJson.dump_options = { pretty: true }\nMultiJson.use(:oj)\n", dump, "`MultiJson.dump_options=`", false),
        (old_lock.as_str(), "MultiJson.use(:oj)\n", dump, "multi_json 1.15.0 is not the version modeled", false),
        (LOCK, "Oj.default_options = Oj.default_options.merge({ mode: :compat }, { escape_mode: :ascii })\nMultiJson.use(:oj)\n", dump, "sets more than the mode", false),
        (LOCK, "MultiJson.use(:oj)\n", "class Codec\n  def self.out(v)\n    MultiJSON.dump(v)\n  end\nend\n", "only the `MultiJson` spelling is lowered", true),
        (LOCK, "MultiJson.use(:oj)\n", "class Codec\n  def self.out(v)\n    MultiJson.use(:json_gem)\n    MultiJson.dump(v)\n  end\nend\n", "`MultiJson.use`", true),
    ] {
        let tree = tree(lock, init, lib);
        assert!(roundhouse::ingest::ingest_app_from_tree(tree.clone()).is_err(), "{init} {lib}: strict ingest passed");
        roundhouse::ingest::survey::activate();
        let app = roundhouse::ingest::ingest_app_from_tree(tree).expect("survey ingest");
        let ledger: Vec<String> = roundhouse::ingest::survey::drain().iter().map(|e| e.to_string()).collect();
        assert!(ledger.iter().any(|e| e.contains(expected)), "{init} {lib}: {ledger:#?}");
        assert_eq!(app.library_classes.iter().any(|lc| lc.name.0.as_str() == "MultiJson"), installed, "{init} {lib}");
    }
    // A version this does not model is no refusal while nothing picks the adapter.
    let tree = tree(&old_lock, "# nothing\n", "class Codec\n  def self.out\n    \"MultiJson\"\n  end\nend\n");
    assert!(roundhouse::ingest::ingest_app_from_tree(tree).is_ok());
}
