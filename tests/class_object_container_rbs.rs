//! Container elements retain the distinction between a class and its instances.
use std::path::PathBuf;

#[path = "support/scratch.rs"]
mod scratch;

const SOURCE: &str = r#"class Beacon
  def self.label = "class"
  def label = "instance"
end
module Catalog
  def self.all = [Beacon]
  def self.mapped = [1].map { Beacon }
  def self.copied
    copy = Beacon
    [copy]
  end
  def self.nested = [{ "kind" => Beacon }]
  def self.instances = [Beacon.new]
  def self.mixed(flag) = [flag ? Beacon : Beacon.new]
  def self.tuple = [Beacon, Beacon.new]
  def self.keyed = { Beacon => "class" }
  def self.written
    result = {}
    result["class"] = Beacon
    result
  end
  def self.pushed
    result = []
    result << Beacon
    result
  end
  def self.memo_array = all.each_with_object([]) { |entry, result| result << entry }
  def self.class_keys = [1].each_with_object({}) { |entry, result| result[Beacon] = "class" }
  def self.registry
    @registry ||= all.each_with_object({}) do |entry, result|
      result[entry.label] = entry
    end.freeze
  end
  def self.named(name) = registry[name.to_s]
  def self.description(name) = named(name).label
  def self.unknown(value) = [value]
  def self.run
    [all.first.label, mapped.first.label, copied.first.label,
      written["class"].label, description("class"), instances.first.label]
  end
end
"#;

fn emit(source: &str) -> Vec<roundhouse::emit::EmittedFile> {
    let tree = [
        ("db/schema.rb", "ActiveRecord::Schema.define do\nend\n"),
        ("app/services/catalog.rb", source),
    ].into_iter().map(|(p, s)| (PathBuf::from(p), s.as_bytes().to_vec())).collect();
    let mut app = roundhouse::ingest::ingest_app_from_tree(tree).unwrap();
    roundhouse::session::analyze_and_lower(&mut app);
    roundhouse::emit::ruby::emit_library(&app)
}

fn rbs(source: &str) -> String {
    emit(source).into_iter().filter(|f| f.path.extension().is_some_and(|e| e == "rbs"))
        .map(|f| f.content).collect::<Vec<_>>().join("\n")
}

#[test]
fn literals_maps_and_writes_keep_class_elements() {
    let rbs = rbs(SOURCE);
    for expected in [
        "def self.all: () -> Array[singleton(Beacon)]",
        "def self.mapped: () -> Array[singleton(Beacon)]",
        "def self.copied: () -> Array[singleton(Beacon)]",
        "def self.nested: () -> Array[Hash[String, singleton(Beacon)]]",
        "def self.keyed: () -> Hash[singleton(Beacon), String]",
        "def self.written: () -> Hash[untyped, singleton(Beacon)]",
        "def self.pushed: () -> Array[singleton(Beacon)]",
        "def self.memo_array: () -> Array[singleton(Beacon)]",
        "def self.class_keys: () -> Hash[singleton(Beacon), String]",
        "def self.registry: () -> Hash[String, singleton(Beacon)]",
        "def self.named: (String name) -> singleton(Beacon)?",
        "def self.instances: () -> Array[Beacon]",
        "def self.mixed: (untyped flag) -> Array[(Beacon | singleton(Beacon))]",
        "def self.tuple: () -> [singleton(Beacon), Beacon]",
        "def self.unknown: (untyped value) -> Array[untyped]",
    ] {
        assert!(rbs.contains(expected), "missing {expected}\n{rbs}");
    }
}

const MAP: &str = "  def self.mapped = [1].map { Beacon }";

#[test]
fn next_values_join_the_tail_without_losing_class_identity() {
    for body in [
        "next Beacon.new if flag; Beacon",
        "next Beacon if flag; Beacon.new",
    ] {
        let source = SOURCE.replace(MAP, &format!("  def self.mapped = [true, false].map {{ |flag| {body} }}"));
        let rbs = rbs(&source);
        assert!(rbs.contains("def self.mapped: () -> Array[(Beacon | singleton(Beacon))]"), "{rbs}");
    }
    for (value, expected) in [("", "singleton(Beacon)?"), ("false", "(bool | singleton(Beacon))")] {
        let source = SOURCE.replace(MAP, &format!("  def self.mapped = [true, false].map {{ |flag| next {value} if flag; Beacon }}"));
        let rbs = rbs(&source);
        assert!(rbs.contains(&format!("def self.mapped: () -> Array[{expected}]")), "{rbs}");
    }
    let source = SOURCE.replace(MAP,
        "  def self.mapped = [1].map { unused = -> { next Beacon.new }; Beacon }");
    assert!(rbs(&source).contains("def self.mapped: () -> Array[singleton(Beacon)]"));
}

#[test]
fn filter_map_keeps_the_truthy_class_value() {
    let source = SOURCE.replace(MAP,
        "  def self.mapped = [true, false].filter_map { |flag| flag && Beacon }");
    let rbs = rbs(&source);
    assert!(rbs.contains("def self.mapped: () -> Array[singleton(Beacon)]"), "{rbs}");
}

fn map_sources() -> Vec<(String, &'static str)> {
    vec![
        (SOURCE.replace(MAP, "  def self.mapped = [true, false].map { |flag| next Beacon.new if flag; Beacon }"),
            "class\ninstance\nclass\nclass\nclass\ninstance\n"),
        (SOURCE.replace(MAP, "  def self.mapped = [true, false].map { |flag| next false if flag; Beacon }")
            .replace("mapped.first.label", "mapped.first.to_s"),
            "class\nfalse\nclass\nclass\nclass\ninstance\n"),
        (SOURCE.replace(MAP, "  def self.mapped = [true, false].map { |flag| next if flag; Beacon }")
            .replace("mapped.first.label", "mapped.first.to_s"),
            "class\n\nclass\nclass\nclass\ninstance\n"),
        (SOURCE.replace(MAP, "  def self.mapped = [true, false].filter_map { |flag| flag && Beacon }"),
            "class\nclass\nclass\nclass\nclass\ninstance\n"),
        (SOURCE.replace("mapped.first.label", "class_keys.keys.first.label"),
            "class\nclass\nclass\nclass\nclass\ninstance\n"),
    ]
}

fn overridden_source() -> String {
    let overrides = r#"class Selection
  def map = Beacon.new
  def each_with_object(seed) = "own memo"
end
class InheritedSelection < Selection
end
"#;
    SOURCE.replace("module Catalog", &format!("{overrides}\nmodule Catalog"))
        .replace("  def self.all", "  def self.custom = InheritedSelection.new.map { Beacon }\n  def self.custom_memo = InheritedSelection.new.each_with_object({}) { |entry, memo| memo[\"class\"] = Beacon }\n  def self.all")
        .replace("written[\"class\"].label, description(\"class\"), instances.first.label]",
            "written[\"class\"].label, description(\"class\"), instances.first.label, custom.label, custom_memo]")
}

#[test]
fn inherited_app_overrides_keep_their_own_return_types() {
    let rbs = rbs(&overridden_source());
    assert!(rbs.contains("def self.custom: () -> Beacon"), "{rbs}");
    assert!(rbs.contains("def self.custom_memo: () -> String"), "{rbs}");
}

#[test]
fn rooted_constants_and_block_copies_keep_the_same_elements() {
    let source = SOURCE.replace("[Beacon]", "[::Beacon]")
        .replace("{ Beacon }", "{ copy = ::Beacon; copy }")
        .replace("= entry\n", "= (copy = entry)\n");
    let rbs = rbs(&source);
    for method in ["all", "mapped", "copied"] {
        assert!(rbs.contains(&format!("def self.{method}: () -> Array[singleton(Beacon)]")), "{rbs}");
    }
    assert!(rbs.contains("def self.registry: () -> Hash[String, singleton(Beacon)]"), "{rbs}");
}

const BOOLEAN_SOURCE: &str = r#"class Beacon
  def self.label = "class"
  def label = "instance"
end
class Other
  def self.label = "other"
end
module Catalog
  def self.fallback = [(nil || Beacon), "done"]
  def self.short_circuit = [(Beacon || nil), "done"]
  def self.guarded(flag) = [(flag && Beacon), "done"]
  def self.nested(flag) = [(flag ? Beacon : nil || Other), "done"]
  def self.coalesced(flag) = [((flag ? Beacon : nil) || Other), "done"]
  def self.mixed(flag) = [((flag ? Beacon : Beacon.new) || Other), "done"]
  def self.unreachable = [(nil && Beacon), "done"]
  def self.false_fallback = [(false || Beacon), "done"]
  def self.true_guard = [(true && Beacon), "done"]
  def self.false_guard = [(false && Beacon), "done"]
  def self.array = [nil || Beacon]
  def self.hash = { (nil || Beacon) => (Beacon || nil) }
  def self.unknown(value) = [value || Beacon]
  def self.unknown_and(value) = [value && Beacon]
  def self.unknown_right(value) = [Beacon && value]
  def self.short_unknown(value) = [Beacon || value]
  def self.run
    [fallback.first.label, short_circuit.first.label,
      guarded(true).first.label, guarded(false).first.to_s,
      nested(true).first.label, nested(false).first.label,
      mixed(true).first.label, mixed(false).first.label,
      unreachable.first.to_s, false_fallback.first.label,
      array.first.label, hash.keys.first.label]
  end
end
"#;

#[test]
fn boolean_operands_keep_reachable_class_values() {
    let rbs = rbs(BOOLEAN_SOURCE);
    for expected in [
        "def self.fallback: () -> [singleton(Beacon), String]",
        "def self.short_circuit: () -> [singleton(Beacon), String]",
        "def self.guarded: (bool flag) -> [(bool | singleton(Beacon)), String]",
        "def self.nested: (bool flag) -> [(singleton(Beacon) | singleton(Other)), String]",
        "def self.coalesced: (untyped flag) -> [(singleton(Beacon) | singleton(Other)), String]",
        "def self.mixed: (bool flag) -> [(Beacon | singleton(Beacon)), String]",
        "def self.unreachable: () -> [nil, String]",
        "def self.false_fallback: () -> [singleton(Beacon), String]",
        "def self.true_guard: () -> [singleton(Beacon), String]",
        "def self.false_guard: () -> [bool, String]",
        "def self.array: () -> Array[singleton(Beacon)]",
        "def self.hash: () -> Hash[singleton(Beacon), singleton(Beacon)]",
        "def self.unknown: (untyped value) -> Array[untyped]",
        "def self.unknown_and: (untyped value) -> Array[untyped]",
        "def self.unknown_right: (untyped value) -> Array[untyped]",
        "def self.short_unknown: (untyped value) -> Array[singleton(Beacon)]",
    ] {
        assert!(rbs.contains(expected), "missing {expected}\n{rbs}");
    }
    let spelled = self::rbs(&BOOLEAN_SOURCE.replace("[nil || Beacon]", "[(nil or ::Beacon)]")
        .replace("(flag && Beacon)", "(flag and ::Beacon)"));
    assert!(spelled.contains("def self.array: () -> Array[singleton(Beacon)]"), "{spelled}");
    assert!(spelled.contains("def self.guarded: (bool flag) -> [(bool | singleton(Beacon)), String]"), "{spelled}");
}

const BOOLEAN_OUTPUT: &str = "class\nclass\nclass\nfalse\nclass\nother\nclass\ninstance\n\nclass\nclass\nclass\n";

const RETURN_SOURCE: &str = r#"class Beacon
  def self.label = "class"
  def label = "instance"
end
module Catalog
  def self.tuple(flag) = [(flag ? (return [Beacon.new, "early"]) : Beacon), "done"]
  def self.array(flag) = [(flag ? (return [Beacon.new]) : Beacon)]
  def self.hashed(flag) = { "item" => (flag ? (return { "item" => Beacon.new }) : Beacon) }
  def self.keyed(flag) = { (flag ? (return { Beacon.new => "early" }) : Beacon) => "done" }
  def self.boolean(flag) = [((flag ? (return [Beacon.new, "early"]) : nil) || Beacon), "done"]
  def self.instance(flag) = [(flag ? (return [Beacon.new, "early"]) : Beacon.new), "done"]
  def self.method_return(flag)
    return Beacon.new if flag
    Beacon
  end
  def self.method_class(flag)
    return Beacon if flag
    Beacon.new
  end
  def self.run
    [tuple(false).first.label, tuple(true).first.label,
      array(false).first.label, array(true).first.label,
      hashed(false)["item"].label, hashed(true)["item"].label,
      keyed(false).keys.first.label, keyed(true).keys.first.label,
      boolean(false).first.label, boolean(true).first.label,
      instance(false).first.label, instance(true).first.label,
      method_return(false).label, method_return(true).label,
      method_class(false).label, method_class(true).label]
  end
end
"#;

#[test]
fn enclosing_returns_do_not_become_container_elements() {
    let rbs = rbs(RETURN_SOURCE);
    for expected in [
        "def self.tuple: (bool flag) -> [(Beacon | singleton(Beacon)), String]",
        "def self.array: (bool flag) -> Array[(Beacon | singleton(Beacon))]",
        "def self.hashed: (bool flag) -> Hash[String, (Beacon | singleton(Beacon))]",
        "def self.keyed: (bool flag) -> Hash[(Beacon | singleton(Beacon)), String]",
        "def self.boolean: (bool flag) -> [(Beacon | singleton(Beacon)), String]",
        "def self.instance: (bool flag) -> [Beacon, String]",
        "def self.method_return: (bool flag) -> (Beacon | singleton(Beacon))",
        "def self.method_class: (bool flag) -> (Beacon | singleton(Beacon))",
    ] {
        assert!(rbs.contains(expected), "missing {expected}\n{rbs}");
    }
}

const RETURN_OUTPUT: &str = "class\ninstance\nclass\ninstance\nclass\ninstance\nclass\ninstance\nclass\ninstance\ninstance\ninstance\nclass\ninstance\ninstance\nclass\n";

fn hoisted_return_source() -> String {
    let mut source = RETURN_SOURCE.to_string();
    for (name, early, tail) in [
        ("tuple", "[Beacon.new, \"early\"]", "[Beacon, \"done\"]"),
        ("array", "[Beacon.new]", "[Beacon]"),
        ("hashed", "{ \"item\" => Beacon.new }", "{ \"item\" => Beacon }"),
        ("keyed", "{ Beacon.new => \"early\" }", "{ Beacon => \"done\" }"),
        ("boolean", "[Beacon.new, \"early\"]", "[(nil || Beacon), \"done\"]"),
        ("instance", "[Beacon.new, \"early\"]", "[Beacon.new, \"done\"]"),
    ] {
        let line = source.lines().find(|line| line.starts_with(&format!("  def self.{name}("))).unwrap().to_string();
        source = source.replace(&line, &format!("  def self.{name}(flag)\n    return {early} if flag\n    {tail}\n  end"));
    }
    source
}

#[test]
fn return_containers_run_as_emitted_ruby() {
    run(RETURN_SOURCE, false, RETURN_OUTPUT);
    run(&hoisted_return_source(), false, RETURN_OUTPUT);
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn return_sidecars_seed_equivalent_hoisted_bodies_on_spinel() {
    // Spinel refuses ReturnNode in literal expressions. Keep the original
    // generated sidecars and exercise them with equivalent statement returns.
    let mut files = emit(RETURN_SOURCE);
    let hoisted = emit(&hoisted_return_source());
    for file in &mut files {
        if file.path.extension().is_some_and(|e| e == "rb") {
            file.content = hoisted.iter().find(|other| other.path == file.path).unwrap().content.clone();
        }
    }
    run_emitted(files, true, RETURN_OUTPUT);
}

#[test]
fn boolean_containers_run_as_emitted_ruby() {
    run(BOOLEAN_SOURCE, false, BOOLEAN_OUTPUT);
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn boolean_containers_run_on_spinel_with_generated_sidecars() {
    run(BOOLEAN_SOURCE, true, BOOLEAN_OUTPUT);
}

fn run(source: &str, native: bool, expected: &str) {
    run_emitted(emit(source), native, expected);
}

fn run_emitted(files: Vec<roundhouse::emit::EmittedFile>, native: bool, expected: &str) {
    let dir = scratch::dir("class_container");
    let mut script = String::new();
    for file in files {
        let path = dir.join(&file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &file.content).unwrap();
        if path.extension().is_some_and(|e| e == "rb") {
            script.push_str(&format!("require_relative {:?}\n", file.path.to_string_lossy()));
        }
    }
    script.push_str("puts Catalog.run\n");
    std::fs::write(dir.join("contract.rb"), script).unwrap();
    let output = if native {
        let compiler = std::env::var("SPINEL").unwrap_or_else(|_| "spinel".into());
        let built = std::process::Command::new(compiler)
            .args(["--rbs", "sig", "contract.rb", "-o", "contract"])
            .current_dir(&dir).output().unwrap();
        assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
        std::process::Command::new(dir.join("contract")).current_dir(&dir).output().unwrap()
    } else {
        std::process::Command::new("ruby").arg("contract.rb").current_dir(&dir).output().unwrap()
    };
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn containers_run_as_emitted_ruby() {
    run(SOURCE, false, "class\nclass\nclass\nclass\nclass\ninstance\n");
    run(&overridden_source(), false, "class\nclass\nclass\nclass\nclass\ninstance\ninstance\nown memo\n");
    for (source, expected) in map_sources() {
        run(&source, false, expected);
    }
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn containers_run_on_spinel_with_generated_sidecars() {
    run(SOURCE, true, "class\nclass\nclass\nclass\nclass\ninstance\n");
    run(&overridden_source(), true, "class\nclass\nclass\nclass\nclass\ninstance\ninstance\nown memo\n");
    for (source, expected) in map_sources() {
        run(&source, true, expected);
    }
}
