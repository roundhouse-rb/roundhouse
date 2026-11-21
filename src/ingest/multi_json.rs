//! multi_json over its oj adapter, lowered into a Ruby stand-in.
//!
//! An app that picks the oj adapter in an initializer (`MultiJson.use(:oj)`,
//! discourse's `config/initializers/100-oj.rb`) gets oj's compat dump:
//! `MultiJson.dump(obj)` is `Oj.dump(obj, mode: :compat, time_format: :ruby,
//! use_to_json: true)`, whatever `Oj.default_options` sets the mode to. For
//! the values the stand-in writes that is:
//!
//! - a Hash's keys as their `to_s`, Strings and Symbols as JSON strings with
//!   `<>&` and U+2028 raw (no Rails escaping);
//! - an integral Float within 2**63 as `<integer>.0`, `-0.0` as `0.0`, any
//!   other with C's `%.16g` (`0.1 + 0.2` is `0.3`, `1e20` is `1e+20`), and
//!   `NaN` and the infinities written as such;
//! - a Time as its `to_json`, Rails' `xmlschema(3)`;
//! - `pretty: <anything>` (oj reads the key, not its value) as
//!   `JSON.pretty_generate` lays it out.
//!
//! Any other value raises, and the analysis refuses a call whose value is
//! not one of those (`analyze::body`'s `multi_json_dump_refusal`).
//!
//! Without such an initializer multi_json 1.21 picks json_gem (`JSON::Ext::
//! Parser` is always loaded), which this does not model: `MultiJson` stays
//! unknown and its uses stay errors. So does an adapter set anywhere but an
//! initializer's top level, and an `Oj.default_options` that sets more than
//! the mode (`float_precision:`, `escape_mode:`, … reach the dump).
//!
//! `MultiJson.load` is refused: oj's strict parser answers a BigDecimal for a
//! decimal of more than 15 digits and reads `01` as 1, where `JSON.parse`
//! answers a Float and raises. Each other call is a refusal of its own,
//! through `survey::continue_or_fail`.
//!
//! Checked against multi_json 1.21.1 and oj 3.17.6 with ActiveSupport.
//!
//! Ruby, JRuby and Spinel emit this class (`lower::oj` gates both stand-ins).

use crate::App;
use crate::dialect::{LibraryClass, LibraryClassOrigin};

use super::IngestError;
use super::survey;
use crate::ingest::prism_api::PrismLocation;

const RECEIVERS: [&str; 4] = ["MultiJson", "::MultiJson", "MultiJSON", "::MultiJSON"];

pub(super) fn lower_multi_json(
    app: &mut App,
    sources: &[crate::span::SourceFile],
    initializers: &[String],
) -> super::IngestResult<()> {
    if !sources.iter().any(|s| s.text.contains("MultiJson") || s.text.contains("MultiJSON")) {
        return Ok(());
    }
    let Some(lock) = app.gem_lock.as_ref() else { return Ok(()) };
    if !lock.has("multi_json") {
        return Ok(());
    }
    let refuse = |message: String| IngestError::Unsupported { file: "config/initializers".into(), message };
    let mut adapters = Vec::new();
    let mut refusals = Vec::new();
    for text in initializers.iter().filter(|t| t.contains("MultiJson") || t.contains("MultiJSON") || t.contains("Oj")) {
        for call in configuration_calls(text) {
            match call {
                Configuration::Adapter { name, top: true } => adapters.push(name),
                Configuration::Adapter { name, top: false } => refusals.push(refuse(format!(
                    "MultiJson not lowered: adapter `{name}` not chosen at an initializer's top level"
                ))),
                Configuration::Other(what) => refusals.push(refuse(format!("MultiJson not lowered: {what}"))),
            }
        }
    }
    if adapters.is_empty() && refusals.is_empty() {
        return Ok(());
    }
    if lock.version_of("multi_json").is_none_or(|v| !v.starts_with("1.21.")) {
        return survey::continue_or_fail(refuse(format!(
            "MultiJson not lowered: multi_json {} is not the version modeled (1.21)",
            lock.version_of("multi_json").unwrap_or("?")
        )));
    }
    if !lock.has("oj") || adapters.iter().any(|a| a != "oj") || adapters.len() != 1 {
        refusals.push(refuse(format!("MultiJson not lowered: adapter {adapters:?}; only the oj adapter is modeled")));
    }
    if !refusals.is_empty() {
        for err in refusals {
            survey::continue_or_fail(err)?;
        }
        return Ok(());
    }
    for source in sources.iter().filter(|s| s.text.contains("MultiJson") || s.text.contains("MultiJSON")) {
        let initializer = source.path.starts_with("config/initializers/") || source.path.contains("/config/initializers/");
        for reason in unmodeled_calls(&source.text, initializer) {
            survey::continue_or_fail(IngestError::Unsupported {
                file: source.path.clone(),
                message: format!("MultiJson not lowered: {reason}"),
            })?;
        }
    }
    if app.library_classes.iter().any(|lc| lc.name.0.as_str() == "MultiJson") {
        return Ok(());
    }
    app.library_classes.extend(stand_ins());
    Ok(())
}

enum Configuration {
    Adapter { name: String, top: bool },
    Other(String),
}

fn text_of(node: &ruby_prism::Node<'_>) -> String {
    String::from_utf8_lossy(node.location().as_slice()).to_string()
}

fn literal_name(node: &ruby_prism::Node<'_>) -> Option<String> {
    if let Some(sym) = node.as_symbol_node() {
        return Some(String::from_utf8_lossy(sym.unescaped()).to_string());
    }
    node.as_string_node().map(|s| String::from_utf8_lossy(s.unescaped()).to_string())
}

/// What an initializer configures of multi_json and of the oj it dumps with.
fn configuration_calls(text: &str) -> Vec<Configuration> {
    struct Finder {
        // Whole ranges: a modifier (`MultiJson.use(:oj) if …`) starts where its call does.
        top: Vec<(usize, usize)>,
        found: Vec<Configuration>,
    }
    impl<'pr> ruby_prism::Visit<'pr> for Finder {
        fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
            let receiver = node.receiver().map(|r| text_of(&r)).unwrap_or_default();
            let method = String::from_utf8_lossy(node.name().as_slice()).to_string();
            let args: Vec<ruby_prism::Node<'_>> =
                node.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
            if RECEIVERS.contains(&receiver.as_str()) {
                match method.as_str() {
                    "use" | "adapter=" | "engine=" => {
                        let at = (node.location().start_offset(), node.location().end_offset());
                        let name = match args.as_slice() {
                            [arg] if node.block().is_none() => literal_name(arg),
                            _ => None,
                        };
                        self.found.push(match name {
                            Some(name) => Configuration::Adapter { name, top: self.top.contains(&at) },
                            None => Configuration::Other(format!("`{}`", text_of(&node.as_node()))),
                        });
                    }
                    "dump_options=" | "load_options=" | "default_options=" | "generate_options=" | "parse_options=" | "default_adapter=" => {
                        self.found.push(Configuration::Other(format!("`{receiver}.{method}`")));
                    }
                    _ => {}
                }
            } else if matches!(receiver.as_str(), "Oj" | "::Oj") && method == "default_options=" {
                // Only the mode, which the adapter's own `mode: :compat` overrides on every dump.
                let options = args.into_iter().next().and_then(|a| match a.as_call_node() {
                    Some(merge) if String::from_utf8_lossy(merge.name().as_slice()) == "merge"
                        && merge.receiver().is_some_and(|r| matches!(text_of(&r).as_str(), "Oj.default_options" | "::Oj.default_options")) =>
                        // One operand: a second merged after it may set more than the mode.
                        merge.arguments().and_then(|m| {
                            let operands: Vec<ruby_prism::Node<'_>> = m.arguments().iter().collect();
                            if operands.len() == 1 { operands.into_iter().next() } else { None }
                        }),
                    _ => Some(a),
                });
                let only_mode = options.as_ref().is_some_and(|o| {
                    let elements: Vec<ruby_prism::Node<'_>> = if let Some(h) = o.as_keyword_hash_node() {
                        h.elements().iter().collect()
                    } else if let Some(h) = o.as_hash_node() {
                        h.elements().iter().collect()
                    } else {
                        return false;
                    };
                    elements.iter().all(|e| e.as_assoc_node().is_some_and(|p| literal_name(&p.key()).as_deref() == Some("mode")))
                });
                if !only_mode {
                    self.found.push(Configuration::Other(format!("`{}` sets more than the mode", text_of(&node.as_node()))));
                }
            }
            ruby_prism::visit_call_node(self, node);
        }
    }
    let parsed = super::prism::parse_silent(text.as_bytes());
    let root = parsed.node();
    let top: Vec<(usize, usize)> = root
        .as_program_node()
        .map(|p| p.statements().body().iter().map(|s| (s.location().start_offset(), s.location().end_offset())).collect())
        .unwrap_or_default();
    let mut finder = Finder { top, found: Vec::new() };
    ruby_prism::Visit::visit(&mut finder, &root);
    finder.found
}

/// The `MultiJson.*` calls in `text` this does not model, described.
fn unmodeled_calls(text: &str, initializer: bool) -> Vec<String> {
    struct Finder {
        found: Vec<String>,
        initializer: bool,
    }
    impl<'pr> ruby_prism::Visit<'pr> for Finder {
        fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
            let receiver = node.receiver().map(|r| text_of(&r)).unwrap_or_default();
            if RECEIVERS.contains(&receiver.as_str()) {
                let method = String::from_utf8_lossy(node.name().as_slice()).to_string();
                let args: Vec<ruby_prism::Node<'_>> =
                    node.arguments().map(|a| a.arguments().iter().collect()).unwrap_or_default();
                match method.as_str() {
                    // The initializer's own call, read above; anywhere else it would switch the adapter as the app runs.
                    "use" | "adapter=" | "engine=" if self.initializer => {}
                    // The stand-in is `MultiJson` only: the analysis resolves no constant alias in a stand-in.
                    _ if receiver.trim_start_matches("::") == "MultiJSON" => {
                        self.found.push(format!("`{receiver}.{method}`: only the `MultiJson` spelling is lowered"));
                    }
                    "dump" | "generate" | "encode" => {
                        // The object is the first argument, a braceless Hash included; a second is the options.
                        let options_ok = match args.get(1) {
                            None => true,
                            Some(options) => {
                                let elements: Option<Vec<ruby_prism::Node<'_>>> = options
                                    .as_keyword_hash_node()
                                    .map(|h| h.elements().iter().collect())
                                    .or_else(|| options.as_hash_node().map(|h| h.elements().iter().collect()));
                                elements.is_some_and(|elements| {
                                    elements.iter().all(|e| e.as_assoc_node().is_some_and(|p| literal_name(&p.key()).as_deref() == Some("pretty")))
                                })
                            }
                        };
                        if args.is_empty() || args.len() > 2 || !options_ok || node.block().is_some() {
                            self.found.push(format!("`{}`", text_of(&node.as_node()).lines().next().unwrap_or_default()));
                        }
                    }
                    "load" | "decode" | "parse" => self.found.push(format!(
                        "`{receiver}.{method}`: oj's strict parser answers a BigDecimal for a decimal of more than 15 digits and reads `01` as 1, which `JSON.parse` does not"
                    )),
                    other => self.found.push(format!("`{receiver}.{other}`")),
                }
            }
            ruby_prism::visit_call_node(self, node);
        }
    }
    let parsed = super::prism::parse_silent(text.as_bytes());
    let mut finder = Finder { found: Vec::new(), initializer };
    ruby_prism::Visit::visit(&mut finder, &parsed.node());
    finder.found
}

/// What the lowered calls reach once multi_json is not there.
fn stand_ins() -> Vec<LibraryClass> {
    const SOURCE: &str = r#"module MultiJson
  # Not `LoadError = ParseError` as the gem has it: the analysis resolves no constant alias in a stand-in. The gem
  # raises only ParseError, which a rescue of either name still catches.
  class DecodeError < StandardError
  end

  class LoadError < DecodeError
  end

  class ParseError < LoadError
  end

  def self.dump(object, options = {})
    options.key?(:pretty) ? MultiJson.pretty(object, "") : MultiJson.compact(object)
  end

  def self.generate(object, options = {})
    options.key?(:pretty) ? MultiJson.pretty(object, "") : MultiJson.compact(object)
  end

  def self.encode(object, options = {})
    options.key?(:pretty) ? MultiJson.pretty(object, "") : MultiJson.compact(object)
  end

  def self.compact(value)
    case value
    when Hash
      parts = []
      value.each { |key, item| parts << JSON.generate(key.to_s) + ":" + MultiJson.compact(item) }
      "{" + parts.join(",") + "}"
    when Array then "[" + value.map { |item| MultiJson.compact(item) }.join(",") + "]"
    when String then JSON.generate(value)
    when Symbol then JSON.generate(value.to_s)
    when Float then MultiJson.float(value)
    when Integer then value.to_s
    when nil then "null"
    when true then "true"
    when false then "false"
    when Time then JSON.generate(ActiveSupport::JSON.time_json(value))
    else raise ArgumentError, "MultiJson.dump: #{value.class} values are not supported yet"
    end
  end

  def self.pretty(value, indent)
    inner = indent + "  "
    case value
    when Hash
      return "{}" if value.empty?
      parts = []
      value.each { |key, item| parts << inner + JSON.generate(key.to_s) + ": " + MultiJson.pretty(item, inner) }
      "{\n" + parts.join(",\n") + "\n" + indent + "}"
    when Array
      return "[]" if value.empty?
      "[\n" + value.map { |item| inner + MultiJson.pretty(item, inner) }.join(",\n") + "\n" + indent + "]"
    else MultiJson.compact(value)
    end
  end

  def self.float(value)
    return "NaN" if value.nan?
    return (value > 0 ? "Infinity" : "-Infinity") if value.infinite?
    return "0.0" if value == 0.0
    # Not `to_i`: 2**63 itself is written whole, and it does not fit an Integer every target has.
    return format("%.1f", value) if value == value.floor && value.abs <= 9223372036854775808.0
    written = format("%.16g", value)
    # oj's own fallback: a 17-character result ending so is a rounding artefact, and Ruby's shortest form is written.
    written.length >= 17 && (written.end_with?("0001") || written.end_with?("9999")) ? value.to_s : written
  end

  def self.use(adapter)
    nil
  end
end
"#;
    let mut classes =
        crate::ingest::ingest_library_classes(SOURCE.as_bytes(), "<multi_json>").expect("the MultiJson stand-ins parse");
    for c in &mut classes {
        c.origin = Some(LibraryClassOrigin::MultiJson);
    }
    classes
}
