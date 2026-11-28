//! Of two modules a controller reaches that define the same method, the
//! spliced one is the one Ruby's ancestors find first (#2137).
//!
//! Each `include` puts the module's ancestors directly above the class.
//! When that walk meets a module already there, it continues below it
//! instead of moving it. So among separate statements the later one comes
//! first, while `include A, B` keeps `A` ahead of `B`.
//!
//! A module that extends `ActiveSupport::Concern` defers its own includes
//! of other concerns: they are included into the class first, and then the
//! concern itself. That changes the answer when two concerns share a
//! dependency.
//!
//! The answers are Ruby 4.0.7's, with ActiveSupport 8.1.4.1, for the same
//! modules and includes outside Rails.

#[path = "support/emit_and_run.rs"]
mod emit_and_run;

use roundhouse::project::BuildTarget;

const CONCERNS: &[(&str, &str)] = &[
    ("first_label", "module FirstLabel\n  extend ActiveSupport::Concern\n\n  def label\n    \"first\"\n  end\nend\n"),
    ("second_label", "module SecondLabel\n  extend ActiveSupport::Concern\n\n  def label\n    \"second\"\n  end\nend\n"),
    ("third_label", "module ThirdLabel\n  extend ActiveSupport::Concern\n\n  def label\n    \"third\"\n  end\nend\n"),
    ("both_labels", "module BothLabels\n  extend ActiveSupport::Concern\n  include FirstLabel\n  include SecondLabel\nend\n"),
    // Plain modules: A, B, and AB including both.
    ("plain_a", "module PlainA\n  def label\n    \"plain a\"\n  end\nend\n"),
    ("plain_b", "module PlainB\n  def label\n    \"plain b\"\n  end\nend\n"),
    ("plain_ab", "module PlainAb\n  include PlainA\n  include PlainB\n\n  def plain_ab\n    \"plain ab\"\n  end\nend\n"),
    // The same three as concerns.
    ("concern_a", "module ConcernA\n  extend ActiveSupport::Concern\n\n  def label\n    \"concern a\"\n  end\nend\n"),
    ("concern_b", "module ConcernB\n  extend ActiveSupport::Concern\n\n  def label\n    \"concern b\"\n  end\nend\n"),
    ("concern_ab", "module ConcernAb\n  extend ActiveSupport::Concern\n  include ConcernA\n  include ConcernB\nend\n"),
    // A concern including a concern and a plain module.
    ("mixed_plain", "module MixedPlain\n  def label\n    \"mixed plain\"\n  end\nend\n"),
    ("mixed_concern", "module MixedConcern\n  extend ActiveSupport::Concern\n\n  def label\n    \"mixed concern\"\n  end\nend\n"),
    ("mixed_both", "module MixedBoth\n  extend ActiveSupport::Concern\n  include MixedConcern\n  include MixedPlain\nend\n"),
    // X includes C, B, A: its ancestors are [X, A, B, C]. B defines no
    // `label`, only a method of its own.
    ("cursor_a", "module CursorA\n  def label\n    \"cursor a\"\n  end\nend\n"),
    ("cursor_b", "module CursorB\n  def other_label\n    \"cursor b\"\n  end\nend\n"),
    ("cursor_c", "module CursorC\n  def label\n    \"cursor c\"\n  end\nend\n"),
    ("cursor_x", "module CursorX\n  include CursorC\n  include CursorB\n  include CursorA\n\n  def cursor_x\n    \"cursor x\"\n  end\nend\n"),
    // A module the superclass includes, and another.
    ("super_m", "module SuperM\n  def label\n    \"super m\"\n  end\nend\n"),
    ("super_n", "module SuperN\n  def label\n    \"super n\"\n  end\nend\n"),
    // A def written in `included do` is the includer's own.
    ("hook_def", "module HookDef\n  extend ActiveSupport::Concern\n\n  included do\n    def label\n      \"hook\"\n    end\n  end\nend\n"),
    ("plain_def", "module PlainDef\n  extend ActiveSupport::Concern\n\n  def label\n    \"plain\"\n  end\nend\n"),
    // `included do` delegates run in hook order: the last defines it.
    ("delegate_one", "module DelegateOne\n  extend ActiveSupport::Concern\n\n  included do\n    delegate :label, to: :source_one\n  end\nend\n"),
    ("delegate_two", "module DelegateTwo\n  extend ActiveSupport::Concern\n\n  included do\n    delegate :label, to: :source_two\n  end\nend\n"),
    // `self.extend ActiveSupport::Concern` is the same concern.
    ("self_a", "module SelfA\n  self.extend ActiveSupport::Concern\n\n  def label\n    \"self a\"\n  end\nend\n"),
    ("self_b", "module SelfB\n  self.extend ActiveSupport::Concern\n\n  def label\n    \"self b\"\n  end\nend\n"),
    ("self_ab", "module SelfAb\n  self.extend ActiveSupport::Concern\n  include SelfA\n  include SelfB\nend\n"),
    // `prepend` puts its module below the class, ahead of it.
    // Includes ConcernA and ConcernB before it extends ActiveSupport::Concern,
    // so Ruby includes both into LateConcern itself.
    ("late_concern", "module LateConcern\n  include ConcernA\n  include ConcernB\n  extend ActiveSupport::Concern\n\n  def late\n    \"late\"\n  end\nend\n"),
    // A plain module: HookDef's hook runs on it and defines `label` there.
    ("hook_plain_wrapper", "module HookPlainWrapper\n  include HookDef\n  include PlainDef\n\n  def hook_plain_wrapper\n    \"wrapper\"\n  end\nend\n"),
    // Includes ConcernA and ConcernB, then a loop's string extends
    // ActiveSupport::Concern: Ruby runs the includes first.
    ("loop_extend_host", "module LoopExtendHost\n  include ConcernA\n  include ConcernB\n  %w[x].each { |m| class_eval \"extend ::ActiveSupport::Concern\" }\n\n  def loop_extend_host\n    \"loop extend host\"\n  end\nend\n"),
    // The loop's string names a module nested in the host.
    ("loop_host", "module LoopHost\n  module GeneratedLeaf\n    def label\n      \"generated leaf\"\n    end\n  end\n\n  %w[unused].each { |m| class_eval \"include GeneratedLeaf\" }\n\n  def loop_host\n    \"loop host\"\n  end\nend\n"),
    ("prepend_a", "module PrependA\n  def label\n    \"prepended\"\n  end\nend\n"),
    ("prepend_b", "module PrependB\n  def label\n    \"included\"\n  end\nend\n"),
    // A hook that includes a module.
    ("hook_a", "module HookA\n  def label\n    \"hook a\"\n  end\nend\n"),
    ("hook_b", "module HookB\n  def label\n    \"hook b\"\n  end\nend\n"),
    ("hook_wrapper", "module HookWrapper\n  extend ActiveSupport::Concern\n\n  included do\n    include HookA\n  end\nend\n"),
    // A class-side def in `included do` beside an instance def of the same name.
    ("class_hook", "module ClassHook\n  extend ActiveSupport::Concern\n\n  included do\n    def self.label\n      \"class hook\"\n    end\n  end\n\n  def label\n    \"class hook instance\"\n  end\nend\n"),
    // A plain module including HookDef: the hook runs on it, not on its includer.
    ("hook_def_wrapper", "module HookDefWrapper\n  include HookDef\n\n  def wrapper_label\n    \"wrapper\"\n  end\nend\n"),
    // One statement of a concern including two hook-defining concerns.
    ("hook_one", "module HookOne\n  extend ActiveSupport::Concern\n\n  included do\n    def label\n      \"hook one\"\n    end\n  end\nend\n"),
    ("hook_two", "module HookTwo\n  extend ActiveSupport::Concern\n\n  included do\n    def label\n      \"hook two\"\n    end\n  end\nend\n"),
    ("hook_bundle", "module HookBundle\n  extend ActiveSupport::Concern\n  include HookOne, HookTwo\nend\n"),
    // A plain module including DelegateOne: the hook's forwarder is the module's.
    ("delegate_wrapper", "module DelegateWrapper\n  include DelegateOne\n\n  def wrapper_name\n    \"wrapper\"\n  end\nend\n"),
    // A loop's string includes LoopLabelB, then an ordinary `include LoopLabelC`.
    ("loop_label_b", "module LoopLabelB\n  def label\n    \"loop b\"\n  end\nend\n"),
    ("loop_label_c", "module LoopLabelC\n  def label\n    \"loop c\"\n  end\nend\n"),
    ("loop_then_include", "module LoopThenInclude\n  %w[LoopLabelB].each { |m| class_eval \"include #{m}\" }\n  include LoopLabelC\n\n  def host\n    \"host\"\n  end\nend\n"),
];

/// (controller, its include lines, Ruby's `label`).
const CASES: &[(&str, &str, &str)] = &[
    // [Separate, SecondLabel, FirstLabel]
    ("separate", "  include FirstLabel\n  include SecondLabel\n", "second"),
    // [Concern, BothLabels, SecondLabel, FirstLabel]
    ("concern", "  include BothLabels\n", "second"),
    // [MultiArg, FirstLabel, SecondLabel]
    ("multi_arg", "  include FirstLabel, SecondLabel\n", "first"),
    // [Again, BothLabels, SecondLabel, FirstLabel, ThirdLabel]: the last
    // `include FirstLabel` finds it already there and moves nothing.
    ("again", "  include ThirdLabel\n  include BothLabels\n  include FirstLabel\n", "second"),
    // [PlainDiamond, PlainAb, PlainB, PlainA]
    ("plain_diamond", "  include PlainB\n  include PlainAb\n", "plain b"),
    // [ConcernDiamond, ConcernAb, ConcernA, ConcernB]
    ("concern_diamond", "  include ConcernB\n  include ConcernAb\n", "concern a"),
    // [Mixed, MixedBoth, MixedConcern, MixedPlain]
    ("mixed", "  include MixedPlain\n  include MixedBoth\n", "mixed concern"),
    // [Cursor, CursorX, CursorB, CursorA, CursorC]: CursorX's walk meets
    // CursorA below CursorB and CursorB above it; CursorC goes below
    // CursorA, not back up between them.
    ("cursor", "  include CursorA\n  include CursorB\n  include CursorX\n", "cursor a"),
    // [SelfDiamond, SelfAb, SelfA, SelfB], as with `extend`.
    ("self_diamond", "  include SelfB\n  include SelfAb\n", "self a"),
    // [PrependA, Prepended, PrependB]
    ("prepended", "  prepend PrependA\n  include PrependB\n", "prepended"),
    // [HookInclude, HookB, HookA, HookWrapper]: the hook put HookA there,
    // so the last `include HookA` moves nothing.
    ("hook_include", "  include HookWrapper\n  include HookB\n  include HookA\n", "hook b"),
    // [ClassHookDef, PlainDef, ClassHook]: the hook's `def self.label` is
    // class-side, so ClassHook's instance `label` is an ancestor's.
    ("class_hook_def", "  include ClassHook\n  include PlainDef\n", "plain"),
    // [WrappedHook, PlainDef, HookDefWrapper, HookDef]: HookDef's hook
    // defined `label` on HookDefWrapper, below PlainDef.
    ("wrapped_hook", "  include HookDefWrapper\n  include PlainDef\n", "plain"),
    // [BundledHook, HookBundle, HookOne, HookTwo]: `include HookOne, HookTwo`
    // records HookTwo first, so HookOne's hook runs last and its `label` stands.
    ("bundled_hook", "  include HookBundle\n", "hook one"),
    // [RewrappedHook, PlainDef, HookDefWrapper, HookDef]: HookDefWrapper put
    // HookDef in the chain, so the controller's own `include HookDef` runs no hook.
    ("rewrapped_hook", "  include HookDefWrapper\n  include HookDef\n  include PlainDef\n", "plain"),
    // [LateConcern, ConcernB, ConcernA]: LateConcern wasn't a concern yet
    // when it included the two, so they are its ancestors, not deferred.
    ("late_concern", "  include ConcernB\n  include LateConcern\n", "concern b"),
    // [PrependA, PrependOverHook, HookDef]: the prepended module is ahead
    // of the `label` HookDef's hook defines on the controller.
    ("prepend_over_hook", "  prepend PrependA\n  include HookDef\n", "prepended"),
    // [HookPlainWrapper, PlainDef, HookDef]: HookDef's `label` is on
    // HookPlainWrapper, above PlainDef.
    ("hook_plain_wrapper", "  include HookPlainWrapper\n", "hook"),
    // [LoopNested, LoopHost, LoopHost::GeneratedLeaf]
    ("loop_nested", "  include LoopHost\n", "generated leaf"),
    // [LoopExtend, LoopExtendHost, ConcernB, ConcernA]: LoopExtendHost
    // wasn't a concern yet when it included the two.
    ("loop_extend", "  include ConcernB\n  include LoopExtendHost\n", "concern b"),
];

/// Cases that need more than a bare controller: (controller, its source,
/// Ruby's `label`).
const SPECIAL: &[(&str, &str, &str)] = &[
    // [SuperNm, SuperN, SuperParent, SuperM]: SuperM is the superclass's
    // already, so the child's `include SuperM` adds nothing.
    ("super_nm", "class SuperNmController < SuperParentController\n  include SuperN\n  include SuperM\n\n  def show\n    render json: { label: label }\n  end\nend\n", "super n"),
    // [SuperMn, SuperN, SuperParent, SuperM]
    ("super_mn", "class SuperMnController < SuperParentController\n  include SuperM\n  include SuperN\n\n  def show\n    render json: { label: label }\n  end\nend\n", "super n"),
    // HookDef's `included do` defines `label` on the controller itself.
    ("hoisted", "class HoistedController < ApplicationController\n  include HookDef\n  include PlainDef\n\n  def show\n    render json: { label: label }\n  end\nend\n", "hook"),
    // HookDef's hook defines `label` on this parent.
    ("hook_parent", "class HookParentController < ApplicationController\n  include HookDef\n\n  def show\n    render json: { label: label }\n  end\nend\n", "hook"),
    // [InheritedHook, PlainDef, HookParent, HookDef]: the parent has HookDef,
    // so the child's `include HookDef` runs no hook, and PlainDef is found first.
    ("inherited_hook", "class InheritedHookController < HookParentController\n  include PlainDef\n  include HookDef\n\n  def show\n    render json: { label: label }\n  end\nend\n", "plain"),
    // DelegateOne's hook delegates `label` on this parent.
    ("delegate_parent", "class DelegateParentController < ApplicationController\n  include DelegateOne\n\n  def show\n    render json: { label: label }\n  end\n\n  private\n\n  def source_one\n    LabelSource.new(\"source one\")\n  end\n\n  def source_two\n    LabelSource.new(\"source two\")\n  end\nend\n", "source one"),
    // The parent has DelegateOne, so the child's `include DelegateOne` runs
    // no hook: the child's own forwarder is DelegateTwo's.
    ("inherited_delegate", "class InheritedDelegateController < DelegateParentController\n  include DelegateTwo\n  include DelegateOne\n\n  def show\n    render json: { label: label }\n  end\nend\n", "source two"),
    // [WrappedDelegate, DelegateWrapper, DelegateOne]: DelegateOne's hook
    // ran on DelegateWrapper, whose forwarder answers `label`.
    ("wrapped_delegate", "class WrappedDelegateController < ApplicationController\n  include DelegateWrapper\n\n  def show\n    render json: { label: label }\n  end\n\n  private\n\n  def source_one\n    LabelSource.new(\"source one\")\n  end\nend\n", "source one"),
    // [RewrappedDelegate, DelegateTwo, DelegateWrapper, DelegateOne]: the
    // controller's own forwarder, from DelegateTwo's hook, comes first.
    ("rewrapped_delegate", "class RewrappedDelegateController < ApplicationController\n  include DelegateWrapper\n  include DelegateTwo\n\n  def show\n    render json: { label: label }\n  end\n\n  private\n\n  def source_one\n    LabelSource.new(\"source one\")\n  end\n\n  def source_two\n    LabelSource.new(\"source two\")\n  end\nend\n", "source two"),
    // [SuperM, PrependInherited, SuperN, SuperParent, SuperM]: the superclass's
    // SuperM doesn't keep `prepend SuperM` from putting it ahead of the class.
    ("prepend_inherited", "class PrependInheritedController < SuperParentController\n  prepend SuperM\n  include SuperN\n\n  def show\n    render json: { label: label }\n  end\nend\n", "super m"),
    // [ReorderedInherited, SuperPairParent, SuperN, SuperM]: the child's
    // includes of both add nothing, so the superclass's order stands.
    ("reordered_inherited", "class ReorderedInheritedController < SuperPairParentController\n  include SuperN\n  include SuperM\n\n  def show\n    render json: { label: label }\n  end\nend\n", "super n"),
    // [LoopInclude, LoopThenInclude, LoopLabelC, LoopLabelB]: the loop runs
    // where it stands, so the later `include LoopLabelC` is nearer.
    ("loop_include", "class LoopIncludeController < ApplicationController\n  include LoopThenInclude\n\n  def show\n    render json: { label: label }\n  end\nend\n", "loop c"),
    // [LibParentChild, SuperN, SuperLibParent, SuperM]: the superclass is a
    // class in lib/, not a controller, and its SuperM still makes the
    // child's `include SuperM` add nothing.
    ("lib_parent_child", "class LibParentChildController < SuperLibParent\n  include SuperN\n  include SuperM\n\n  def show\n    render json: { label: label }\n  end\nend\n", "super n"),
    // [DeepChain, Deep0, Deep1, …, Deep65]: a chain of 66 modules, each
    // including the next; only the last defines `label`.
    ("deep_chain", "class DeepChainController < ApplicationController\n  include Deep0\n\n  def show\n    render json: { label: label }\n  end\nend\n", "deep 65"),
    // [LibDelegateChild, DelegateLibParent, DelegateOne]: the lib/ superclass
    // has DelegateOne, so the child's include runs no hook; the forwarder
    // DelegateOne's hook defined on the superclass answers.
    ("lib_delegate_child", "class LibDelegateChildController < DelegateLibParent\n  include DelegateOne\n\n  def show\n    render json: { label: label }\n  end\n\n  private\n\n  def source_one\n    LabelSource.new(\"source one\")\n  end\nend\n", "source one"),
    // DelegateTwo's hook runs last, so its `delegate` defines `label`.
    ("delegated", "class DelegatedController < ApplicationController\n  include DelegateOne\n  include DelegateTwo\n\n  def show\n    render json: { label: label }\n  end\n\n  private\n\n  def source_one\n    LabelSource.new(\"source one\")\n  end\n\n  def source_two\n    LabelSource.new(\"source two\")\n  end\nend\n", "source two"),
];

fn camel(snake: &str) -> String {
    snake.split('_').map(|w| w[..1].to_uppercase() + &w[1..]).collect()
}

fn app() -> emit_and_run::Overlay {
    let mut app = emit_and_run::empty_app()
        .write("app/controllers/application_controller.rb", "class ApplicationController < ActionController::API\nend\n");
    for (file, source) in CONCERNS {
        app = app.write(&format!("app/controllers/concerns/{file}.rb"), source);
    }
    app = app
        .write("app/controllers/super_parent_controller.rb", "class SuperParentController < ApplicationController\n  include SuperM\nend\n")
        .write("app/controllers/super_pair_parent_controller.rb", "class SuperPairParentController < ApplicationController\n  include SuperM\n  include SuperN\nend\n")
        .write("lib/super_lib_parent.rb", "class SuperLibParent < ApplicationController\n  include SuperM\nend\n")
        .write("lib/delegate_lib_parent.rb", "class DelegateLibParent < ApplicationController\n  include DelegateOne\nend\n")
        .write("app/models/label_source.rb", "class LabelSource\n  def initialize(label)\n    @label = label\n  end\n\n  attr_reader :label\nend\n");
    // Deep0 includes Deep1, … Deep64 includes Deep65; each has a method of
    // its own, so each is kept.
    for i in 0..66 {
        let body = if i < 65 {
            format!("  include Deep{}\n\n  def deep{i}\n    \"deep {i}\"\n  end\n", i + 1)
        } else {
            "  def label\n    \"deep 65\"\n  end\n".to_string()
        };
        app = app.write(&format!("app/controllers/concerns/deep{i}.rb"), &format!("module Deep{i}\n{body}end\n"));
    }
    let mut routes = String::from("Rails.application.routes.draw do\n");
    for (name, source, _) in SPECIAL {
        app = app.write(&format!("app/controllers/{name}_controller.rb"), source);
        routes.push_str(&format!("  get \"/{name}\" => \"{name}#show\"\n"));
    }
    for (name, includes, _) in CASES {
        app = app.write(
            &format!("app/controllers/{name}_controller.rb"),
            &format!(
                "class {}Controller < ApplicationController\n{includes}\n  def show\n    render json: {{ label: label }}\n  end\nend\n",
                camel(name)
            ),
        );
        routes.push_str(&format!("  get \"/{name}\" => \"{name}#show\"\n"));
    }
    routes.push_str("end\n");
    app.write("config/routes.rb", &routes)
}

#[test]
fn the_spliced_method_is_the_one_rubys_ancestors_find_first() {
    let mut script = String::from(
        r##"def probe(path)
  status, _headers, body = Main.run_rack("REQUEST_METHOD" => "GET", "PATH_INFO" => path, "QUERY_STRING" => "", "rack.input" => StringIO.new(""))
  text = +""
  body.each { |part| text << part }
  [status, text]
end
got = {}
want = {}
"##,
    );
    for (name, _, label) in CASES.iter().chain(SPECIAL) {
        script.push_str(&format!("got[{name:?}] = begin\n  probe(\"/{name}\")\nrescue StandardError => e\n  [e.class.name, e.message[0, 80]]\nend\nwant[{name:?}] = [200, %({{\"label\":{label:?}}})]\n"));
    }
    script.push_str("wrong = got.reject { |k, v| v == want[k] }\nraise \"wrong: #{wrong.inspect}\" unless wrong.empty?\nputs \"include order passed\"\n");
    let run = app().run_ruby(&script);
    run.assert_passes();
    assert!(run.stdout.contains("include order passed"), "{}", run.stdout);
}

/// The app of one controller that includes `include_lines` and renders
/// `method`, with the concerns in `files`. Its private `source_one` and
/// `source_two` answer what a delegated `label` reads.
fn single_app(files: &[(&str, &str)], include_lines: &str, method: &str) -> emit_and_run::Overlay {
    let mut app = emit_and_run::empty_app()
        .write("app/controllers/application_controller.rb", "class ApplicationController < ActionController::API\nend\n")
        .write("app/models/label_source.rb", "class LabelSource\n  def initialize(label)\n    @label = label\n  end\n\n  attr_reader :label\nend\n");
    for (file, source) in files {
        app = app.write(&format!("app/controllers/concerns/{file}.rb"), source);
    }
    app.write(
        "app/controllers/single_controller.rb",
        &format!(
            "class SingleController < ApplicationController\n{include_lines}\n  def show\n    render json: {{ label: {method} }}\n  end\n\n  private\n\n  def source_one\n    LabelSource.new(\"source one\")\n  end\n\n  def source_two\n    LabelSource.new(\"source two\")\n  end\nend\n"
        ),
    )
    .write("config/routes.rb", "Rails.application.routes.draw do\n  get \"/single\" => \"single#show\"\nend\n")
}

/// The controller as `target` emits it, from the app `single_app` makes.
fn emitted_controller(files: &[(&str, &str)], include_lines: &str, target: BuildTarget) -> String {
    let (emitted, errors) = single_app(files, include_lines, "label").emit(target);
    assert!(errors.is_empty(), "{errors:#?}");
    let path = emitted.join("app/controllers/single_controller.rb");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The modules each `include` of the ingested controller names, as the
/// splice and the analyzer read them: after a hook's includes are carried
/// onto the controller's own `include`.
fn ingested_include_lines(files: &[(&str, &str)], include_lines: &str) -> Vec<Vec<String>> {
    use roundhouse::dialect::ControllerBodyItem;
    use roundhouse::expr::ExprNode;
    let mut tree: std::collections::HashMap<std::path::PathBuf, Vec<u8>> = std::collections::HashMap::new();
    tree.insert("app/controllers/application_controller.rb".into(), b"class ApplicationController < ActionController::API\nend\n".to_vec());
    for (file, source) in files {
        tree.insert(format!("app/controllers/concerns/{file}.rb").into(), source.as_bytes().to_vec());
    }
    tree.insert(
        "app/controllers/single_controller.rb".into(),
        format!("class SingleController < ApplicationController\n{include_lines}\n  def show\n    head :ok\n  end\nend\n").into_bytes(),
    );
    let app = roundhouse::ingest::ingest_app_from_tree(tree).expect("ingest");
    let controller = app.controllers.iter().find(|c| c.name.0.as_str() == "SingleController").expect("SingleController");
    controller
        .body
        .iter()
        .filter_map(|item| match item {
            ControllerBodyItem::Unknown { expr, .. } => match &*expr.node {
                ExprNode::Send { recv: None, method, args, .. } if method.as_str() == "include" => Some(
                    args.iter()
                        .map(|arg| match &*arg.node {
                            ExprNode::Const { path } => path.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("::"),
                            other => format!("{other:?}"),
                        })
                        .collect(),
                ),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The body of `def label` in the controller `target` emits, its lines
/// trimmed.
fn spliced_label(files: &[(&str, &str)], include_lines: &str, target: BuildTarget) -> String {
    let controller = emitted_controller(files, include_lines, target);
    let at = controller.find("  def label\n").unwrap_or_else(|| panic!("no spliced label:\n{controller}"));
    let body = &controller[at + "  def label\n".len()..];
    let end = body.find("\n  end\n").unwrap_or_else(|| panic!("label unterminated:\n{controller}"));
    body[..end].trim().to_string()
}

/// One controller reading `method` through the modules in `files`, against
/// Ruby's answer for the same modules.
fn single_case(files: &[(&str, &str)], include_lines: &str, method: &str, want: &str) {
    let app = single_app(files, include_lines, method);
    let script = format!(
        r##"status, _headers, body = begin
  Main.run_rack("REQUEST_METHOD" => "GET", "PATH_INFO" => "/single", "QUERY_STRING" => "", "rack.input" => StringIO.new(""))
rescue StandardError => e
  [e.class.name, {{}}, [e.message[0, 100]]]
end
text = +""
body.each {{ |part| text << part }}
got = [status, text]
want = [200, %({{"label":{want:?}}})]
raise "got #{{got.inspect}}, want #{{want.inspect}}" unless got == want
puts "single case passed"
"##
    );
    let run = app.run_ruby(&script);
    run.assert_passes();
    assert!(run.stdout.contains("single case passed"), "{}", run.stdout);
}

/// A plain module whose body only includes others is still an ancestor:
/// `[Single, PlainOnlyAb, PlainOnlyB, PlainOnlyA]` (#2278).
#[test]
fn a_plain_module_of_includes_only_passes_its_ancestors_on() {
    single_case(
        &[
            ("plain_only_a", "module PlainOnlyA\n  def plain_a_only\n    \"plain a\"\n  end\nend\n"),
            ("plain_only_b", "module PlainOnlyB\n  def label\n    \"plain b\"\n  end\nend\n"),
            ("plain_only_ab", "module PlainOnlyAb\n  include PlainOnlyA\n  include PlainOnlyB\nend\n"),
        ],
        "  include PlainOnlyAb\n",
        "plain_a_only",
        "plain a",
    );
}

/// `self.include` in a concern of includes only is the same include:
/// `[Single, SelfWrapper, HookDef]`, and HookDef's hook defines `label` (#2278).
#[test]
fn a_concern_of_self_includes_only_passes_its_hook_on() {
    single_case(
        &[
            ("hook_def", "module HookDef\n  extend ActiveSupport::Concern\n\n  included do\n    def label\n      \"hook\"\n    end\n  end\nend\n"),
            ("self_wrapper", "module SelfWrapper\n  self.extend ActiveSupport::Concern\n  self.include HookDef\nend\n"),
        ],
        "  include SelfWrapper\n",
        "label",
        "hook",
    );
}

/// `self.included do` is the same hook: its `label` is defined on the
/// controller, ahead of the later PlainDef's (#2278).
#[test]
fn a_self_included_hook_defines_on_the_includer() {
    single_case(
        &[
            ("hook_self", "module HookSelf\n  extend ActiveSupport::Concern\n\n  self.included do\n    def label\n      \"hook\"\n    end\n  end\nend\n"),
            ("plain_def", "module PlainDef\n  extend ActiveSupport::Concern\n\n  def label\n    \"plain\"\n  end\nend\n"),
        ],
        "  include HookSelf\n  include PlainDef\n",
        "label",
        "hook",
    );
}

const HOOK_DEF: (&str, &str) = ("hook_def", "module HookDef\n  extend ActiveSupport::Concern\n\n  included do\n    def label\n      \"hook\"\n    end\n  end\nend\n");
const PLAIN_DEF: (&str, &str) = ("plain_def", "module PlainDef\n  extend ActiveSupport::Concern\n\n  def label\n    \"plain\"\n  end\nend\n");

/// `include B, C, HookWrapper` includes HookWrapper first, and its hook
/// includes B right above it. The explicit B then finds B there and moves
/// nothing, and C goes above them: `[Single, C, B, HookWrapper]` (#2278).
#[test]
fn a_hook_include_goes_in_before_the_explicit_arguments_left_of_it() {
    let files = [
        ("b", "module B\n  def label\n    \"b\"\n  end\nend\n"),
        ("c", "module C\n  def label\n    \"c\"\n  end\nend\n"),
        ("hook_wrapper", "module HookWrapper\n  extend ActiveSupport::Concern\n\n  included do\n    include B\n  end\nend\n"),
    ];
    let lines = "  include B, C, HookWrapper\n";
    // The explicit B stays beside the one the hook carries: dropping either
    // would put B back ahead of C.
    assert_eq!(ingested_include_lines(&files, lines), [["B", "C", "B", "HookWrapper"]]);
    assert_eq!(spliced_label(&files, lines, BuildTarget::Spinel), "\"c\"");
    single_case(&files, lines, "label", "c");
}

/// The hook's `label` is defined on the controller when it is included. A
/// later opening's `label` is the module's own, below the controller, so the
/// hook's answers (#2278).
#[test]
fn a_hook_def_stands_over_a_later_ordinary_def_of_its_name() {
    let files = [(
        "hook_reopen",
        "module HookReopen\n  extend ActiveSupport::Concern\n\n  included do\n    def label\n      \"hook\"\n    end\n  end\nend\n\nmodule HookReopen\n  def label\n    \"ordinary\"\n  end\nend\n",
    )];
    let lines = "  include HookReopen\n";
    assert_eq!(spliced_label(&files, lines, BuildTarget::Spinel), "\"hook\"");
    single_case(&files, lines, "label", "hook");
}

/// Each plain wrapper runs its concern's hook on itself, so each holds its
/// own forwarder. W3 is the nearest ancestor, and its forwarder reads
/// `source_two` (#2278).
#[test]
fn a_hook_forwarder_stands_on_each_plain_wrapper_it_ran_on() {
    let files = [
        ("delegate_one", "module DelegateOne\n  extend ActiveSupport::Concern\n\n  included { delegate :label, to: :source_one }\nend\n"),
        ("delegate_two", "module DelegateTwo\n  extend ActiveSupport::Concern\n\n  included { delegate :label, to: :source_two }\nend\n"),
        ("w1", "module W1\n  include DelegateOne\nend\n"),
        ("w2", "module W2\n  include DelegateTwo\nend\n"),
        ("w3", "module W3\n  include DelegateTwo\nend\n"),
    ];
    let lines = "  include W2\n  include W1\n  include W3\n";
    assert_eq!(spliced_label(&files, lines, BuildTarget::Spinel), "self.source_two.label");
    single_case(&files, lines, "label", "source two");
}

/// A plain wrapper takes a concern's deferred dependencies itself: HookDef's
/// hook runs on Wrapper, so its `label` is Wrapper's, ahead of PlainDef's
/// below (#2278).
#[test]
fn a_deferred_hook_runs_on_the_plain_wrapper() {
    let files = [
        HOOK_DEF,
        PLAIN_DEF,
        ("hook_bundle", "module HookBundle\n  extend ActiveSupport::Concern\n\n  include HookDef\nend\n"),
        ("wrapper", "module Wrapper\n  include HookBundle\n  include PlainDef\nend\n"),
    ];
    let lines = "  include Wrapper\n";
    assert_eq!(spliced_label(&files, lines, BuildTarget::Spinel), "\"hook\"");
    single_case(&files, lines, "label", "hook");
}

/// HookReopen's hook defines `label` on the includer, and a later opening's
/// ordinary `label` is the module's own, below it.
const HOOK_REOPEN: (&str, &str) = (
    "hook_reopen",
    "module HookReopen\n  extend ActiveSupport::Concern\n\n  included do\n    def label\n      \"hook\"\n    end\n  end\nend\n\nmodule HookReopen\n  def label\n    \"ordinary\"\n  end\nend\n",
);

/// `single_app`'s controller, under `HookLibParent`: a class in lib/ that
/// includes HookReopen.
fn lib_parent_app(child_lines: &str) -> emit_and_run::Overlay {
    emit_and_run::empty_app()
        .write("app/controllers/application_controller.rb", "class ApplicationController < ActionController::API\nend\n")
        .write(&format!("app/controllers/concerns/{}.rb", HOOK_REOPEN.0), HOOK_REOPEN.1)
        .write("lib/hook_lib_parent.rb", "class HookLibParent < ApplicationController\n  include HookReopen\nend\n")
        .write(
            "app/controllers/single_controller.rb",
            &format!("class SingleController < HookLibParent\n{child_lines}\n  def show\n    render json: {{ label: label }}\n  end\nend\n"),
        )
        .write("config/routes.rb", "Rails.application.routes.draw do\n  get \"/single\" => \"single#show\"\nend\n")
}

/// The hook ran on the lib/ superclass, so `label` is HookLibParent's own:
/// `[SingleController, HookLibParent, HookReopen]`. The child's repeated
/// include runs no hook and adds nothing (#2278).
#[test]
fn a_hook_def_that_ran_on_a_lib_superclass_stands_over_the_modules_own() {
    for child_lines in ["  include HookReopen\n", ""] {
        let (emitted, errors) = lib_parent_app(child_lines).emit(BuildTarget::Spinel);
        assert!(errors.is_empty(), "{errors:#?}");
        // The lib/ class holds the hook's `label` once, as its own.
        let parent = std::fs::read_to_string(emitted.join("app/models/hook_lib_parent.rb")).unwrap();
        let lines: Vec<&str> = parent.lines().filter(|l| !l.starts_with("#<SPINEL_SOURCE>")).map(str::trim).collect();
        let defs: Vec<usize> = (0..lines.len()).filter(|&i| lines[i] == "def label").collect();
        assert!(matches!(defs.as_slice(), [at] if lines[at + 1] == "\"hook\""), "{parent}");
        let controller = std::fs::read_to_string(emitted.join("app/controllers/single_controller.rb")).unwrap();
        if let Some(at) = controller.find("  def label\n") {
            assert_eq!(controller[at..].lines().nth(1).map(str::trim), Some("\"hook\""), "{child_lines:?}:\n{controller}");
        }
        let run = lib_parent_app(child_lines).run_ruby(
            r##"status, _headers, body = begin
  Main.run_rack("REQUEST_METHOD" => "GET", "PATH_INFO" => "/single", "QUERY_STRING" => "", "rack.input" => StringIO.new(""))
rescue StandardError => e
  [e.class.name, {}, [e.message[0, 100]]]
end
text = +""
body.each { |part| text << part }
got = [status, text]
want = [200, %({"label":"hook"})]
raise "got #{got.inspect}, want #{want.inspect}" unless got == want
puts "lib parent case passed"
"##,
        );
        run.assert_passes();
        assert!(run.stdout.contains("lib parent case passed"), "{child_lines:?}: {}", run.stdout);
    }
}

/// SingleController under `parent`, a class from `libs`, with `concerns`;
/// against Ruby's answer for `label`, as JSON. The Spinel emit has no errors.
fn lib_case(concerns: &[(&str, &str)], libs: &[(&str, &str)], parent: &str, want: &str) {
    let app = || {
        let mut app = emit_and_run::empty_app()
            .write("app/controllers/application_controller.rb", "class ApplicationController < ActionController::API\nend\n");
        for (file, source) in concerns {
            app = app.write(&format!("app/controllers/concerns/{file}.rb"), source);
        }
        for (file, source) in libs {
            app = app.write(&format!("lib/{file}.rb"), source);
        }
        app.write(
            "app/controllers/single_controller.rb",
            &format!("class SingleController < {parent}\n  def show\n    render json: {{ label: label }}\n  end\nend\n"),
        )
        .write("config/routes.rb", "Rails.application.routes.draw do\n  get \"/single\" => \"single#show\"\nend\n")
    };
    let (_, errors) = app().emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    let run = app().run_ruby(&format!(
        r##"status, _headers, body = begin
  Main.run_rack("REQUEST_METHOD" => "GET", "PATH_INFO" => "/single", "QUERY_STRING" => "", "rack.input" => StringIO.new(""))
rescue StandardError => e
  [e.class.name, {{}}, [e.message[0, 100]]]
end
text = +""
body.each {{ |part| text << part }}
got = [status, text]
want = [200, %({{"label":{want}}})]
raise "got #{{got.inspect}}, want #{{want.inspect}}" unless got == want
puts "lib case passed"
"##
    ));
    run.assert_passes();
    assert!(run.stdout.contains("lib case passed"), "{}", run.stdout);
}

/// BaseParent's hook ran and its own `label` replaced the hook's. The
/// subclass's repeated include adds nothing and runs no hook, so BaseParent's
/// `label` answers (#2278).
#[test]
fn a_lib_class_takes_no_hook_def_for_a_module_its_superclass_has() {
    lib_case(
        &[HOOK_REOPEN],
        &[
            ("base_parent", "class BaseParent < ApplicationController\n  include HookReopen\n\n  def label\n    \"base\"\n  end\nend\n"),
            ("derived_parent", "class DerivedParent < BaseParent\n  include HookReopen\nend\n"),
        ],
        "DerivedParent",
        "\"base\"",
    );
}

/// HookOnly's hook runs after HookReopen's, so its `label` stands: the
/// module lookup finds it already, ahead of HookReopen (#2278).
#[test]
fn a_lib_class_leaves_a_name_two_hooks_define_to_the_modules() {
    lib_case(
        &[HOOK_REOPEN, ("hook_only", "module HookOnly\n  extend ActiveSupport::Concern\n\n  included do\n    def label\n      \"b hook\"\n    end\n  end\nend\n")],
        &[("pair_parent", "class PairParent < ApplicationController\n  include HookReopen\n  include HookOnly\nend\n")],
        "PairParent",
        "\"b hook\"",
    );
}

/// TwiceParent's first opening ran the hook and then replaced its `label`;
/// the second opening's include runs no hook (#2278).
#[test]
fn a_lib_class_takes_no_hook_def_on_a_repeated_include_of_its_own() {
    lib_case(
        &[HOOK_REOPEN],
        &[
            ("twice_parent", "class TwiceParent < ApplicationController\n  include HookReopen\n\n  def label\n    \"base\"\n  end\nend\n"),
            ("twice_parent_again", "class TwiceParent < ApplicationController\n  include HookReopen\nend\n"),
        ],
        "TwiceParent",
        "\"base\"",
    );
}

/// The block's `attr_reader :label` comes after its `def label` and replaces
/// it on the includer: `label` reads the unset `@label` (#2278).
#[test]
fn a_hook_def_stays_below_a_later_reader_of_its_block() {
    lib_case(
        &[(
            "hook_reader",
            "module HookReader\n  extend ActiveSupport::Concern\n\n  included do\n    def label\n      \"hook\"\n    end\n    attr_reader :label\n  end\nend\n\nmodule HookReader\n  def label\n    \"ordinary\"\n  end\nend\n",
        )],
        &[("reader_parent", "class ReaderParent < ApplicationController\n  include HookReader\nend\n")],
        "ReaderParent",
        "null",
    );
}

/// HookReopen with `hook2` and `ordinary2`: a second concern whose hook
/// `label` a later opening's ordinary `label` shadows.
const HOOK_REOPEN2: (&str, &str) = (
    "hook_reopen2",
    "module HookReopen2\n  extend ActiveSupport::Concern\n\n  included do\n    def label\n      \"hook2\"\n    end\n  end\nend\n\nmodule HookReopen2\n  def label\n    \"ordinary2\"\n  end\nend\n",
);

/// The body line of each `def label` the Spinel emit gives the lib/ class
/// in `file`.
fn lib_label_bodies(concerns: &[(&str, &str)], libs: &[(&str, &str)], file: &str) -> Vec<String> {
    let mut app = emit_and_run::empty_app()
        .write("app/controllers/application_controller.rb", "class ApplicationController < ActionController::API\nend\n");
    for (name, source) in concerns {
        app = app.write(&format!("app/controllers/concerns/{name}.rb"), source);
    }
    for (name, source) in libs {
        app = app.write(&format!("lib/{name}.rb"), source);
    }
    let (emitted, errors) = app.emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    let class = std::fs::read_to_string(emitted.join(format!("app/models/{file}.rb"))).unwrap();
    let lines: Vec<&str> = class.lines().filter(|l| !l.starts_with("#<SPINEL_SOURCE>")).map(str::trim).collect();
    (0..lines.len()).filter(|&i| lines[i] == "def label").map(|i| lines[i + 1].to_string()).collect()
}

/// Both hooks define `label` on the class, in the order the includes run,
/// so the last one's stands over both modules' own (#2278). Within one
/// `include A, B`, B is included first, so A's hook runs last.
#[test]
fn a_lib_class_takes_the_last_hooks_def_of_two() {
    for (includes, want) in [
        ("  include HookReopen\n  include HookReopen2\n", "\"hook2\""),
        ("  include HookReopen2\n  include HookReopen\n", "\"hook\""),
        ("  include HookReopen, HookReopen2\n", "\"hook\""),
    ] {
        let libs = [("two_hook_parent", format!("class TwoHookParent < ApplicationController\n{includes}end\n"))];
        let libs: Vec<(&str, &str)> = libs.iter().map(|(f, s)| (*f, s.as_str())).collect();
        assert_eq!(lib_label_bodies(&[HOOK_REOPEN, HOOK_REOPEN2], &libs, "two_hook_parent"), [want], "{includes:?}");
        lib_case(&[HOOK_REOPEN, HOOK_REOPEN2], &libs, "TwoHookParent", want);
    }
}

/// The block's `def label` comes after its `attr_reader :label` and replaces
/// the reader on the includer, whether or not a later opening's ordinary
/// `label` shadows it on the module (#2278).
#[test]
fn a_hook_def_below_its_reader_stands() {
    let hook = "module HookReaderFirst\n  extend ActiveSupport::Concern\n\n  included do\n    attr_reader :label\n    def label\n      \"hook\"\n    end\n  end\nend\n";
    let reopened = format!("{hook}\nmodule HookReaderFirst\n  def label\n    \"ordinary\"\n  end\nend\n");
    for source in [reopened.as_str(), hook] {
        let concern = ("hook_reader_first", source);
        let libs = [("reader_first_parent", "class ReaderFirstParent < ApplicationController\n  include HookReaderFirst\nend\n")];
        assert_eq!(lib_label_bodies(&[concern], &libs, "reader_first_parent"), ["\"hook\""], "{source}");
        lib_case(&[concern], &libs, "ReaderFirstParent", "\"hook\"");
    }
}


/// LastHookOnly's hook defines `label` on the class, and no reopening
/// shadows it on the module.
const LAST_HOOK_ONLY: (&str, &str) = (
    "last_hook_only",
    "module LastHookOnly\n  extend ActiveSupport::Concern\n\n  included do\n    def label\n      \"last\"\n    end\n  end\nend\n",
);

/// A repeated `include` of a module the class has adds nothing and runs no
/// hook again, so LastHookOnly's hook ran last (#2278).
#[test]
fn a_repeated_include_runs_its_hook_once() {
    let libs = [("repeat_parent", "class RepeatParent < ApplicationController\n  include HookReopen2\n  include LastHookOnly\n  include HookReopen2\nend\n")];
    assert!(!lib_label_bodies(&[HOOK_REOPEN2, LAST_HOOK_ONLY], &libs, "repeat_parent").contains(&"\"hook2\"".to_string()));
    lib_case(&[HOOK_REOPEN2, LAST_HOOK_ONLY], &libs, "RepeatParent", "\"last\"");
}

/// The block's second `attr_reader :label` comes after its `def label` and
/// replaces it on the includer, whether or not a reopening shadows the def
/// on the module (#2278).
#[test]
fn a_reader_below_the_hook_def_stands_over_it() {
    let hook = "module ReaderTwice\n  extend ActiveSupport::Concern\n\n  included do\n    attr_reader :label\n    def label\n      \"hook\"\n    end\n    attr_reader :label\n  end\nend\n";
    let reopened = format!("{hook}\nmodule ReaderTwice\n  def label\n    \"ordinary\"\n  end\nend\n");
    for source in [hook, reopened.as_str()] {
        let concern = ("reader_twice", source);
        let libs = [("reader_twice_parent", "class ReaderTwiceParent < ApplicationController\n  include ReaderTwice\nend\n")];
        assert_eq!(lib_label_bodies(&[concern], &libs, "reader_twice_parent"), ["@label"], "{source}");
        lib_case(&[concern], &libs, "ReaderTwiceParent", "null");
    }
}

/// A hook def below its reader that calls `super`, with no reopening: the
/// module keeps the hoisted copy of the same def, which a copy on the class
/// would reach through `super` and run twice. It isn't copied, and the
/// reader stands, as before. Ruby's `"hook:base"` waits on #2278. With a
/// reopening, `super` reaches the module's ordinary def, as in Ruby.
#[test]
fn a_hook_def_calling_super_is_copied_only_over_a_reopening() {
    let hook = "module HookSuper\n  extend ActiveSupport::Concern\n\n  included do\n    attr_reader :label\n    def label\n      \"hook:#{super}\"\n    end\n  end\nend\n";
    let libs = [
        ("super_base", "class SuperBase < ApplicationController\n  def label\n    \"base\"\n  end\nend\n"),
        ("super_hook_parent", "class SuperHookParent < SuperBase\n  include HookSuper\nend\n"),
    ];
    assert_eq!(lib_label_bodies(&[("hook_super", hook)], &libs, "super_hook_parent"), ["@label"]);
    let reopened = format!("{hook}\nmodule HookSuper\n  def label\n    \"ordinary\"\n  end\nend\n");
    lib_case(&[("hook_super", reopened.as_str())], &libs, "SuperHookParent", "\"hook:ordinary\"");
}

const READER_DEF_A: (&str, &str) = (
    "reader_def_a",
    "module ReaderDefA\n  extend ActiveSupport::Concern\n\n  included do\n    attr_reader :label\n    def label\n      \"hook\"\n    end\n  end\nend\n",
);
const READER_ONLY_B: (&str, &str) = ("reader_only_b", "module ReaderOnlyB\n  extend ActiveSupport::Concern\n\n  included do\n    attr_reader :label\n  end\nend\n");

/// A later hook's reader replaces an earlier hook's def on the includer, as
/// the def replaced its own block's reader (#2278).
#[test]
fn a_later_hooks_reader_stands_over_an_earlier_hooks_def() {
    for (includes, body, want) in [
        ("  include ReaderDefA\n  include ReaderOnlyB\n", "@label", "null"),
        ("  include ReaderOnlyB\n  include ReaderDefA\n", "\"hook\"", "\"hook\""),
    ] {
        let libs = [("reader_pair_parent", format!("class ReaderPairParent < ApplicationController\n{includes}end\n"))];
        let libs: Vec<(&str, &str)> = libs.iter().map(|(f, s)| (*f, s.as_str())).collect();
        assert_eq!(lib_label_bodies(&[READER_DEF_A, READER_ONLY_B], &libs, "reader_pair_parent"), [body], "{includes:?}");
        lib_case(&[READER_DEF_A, READER_ONLY_B], &libs, "ReaderPairParent", want);
    }
}

/// ShadowX: a hook `label` ("x hook") that a reopening's ordinary `label`
/// shadows on the module.
fn shadow(x: &str) -> (String, String) {
    let name = format!("Shadow{x}");
    (
        format!("shadow_{}", x.to_lowercase()),
        format!("module {name}\n  extend ActiveSupport::Concern\n\n  included do\n    def label\n      \"{l} hook\"\n    end\n  end\nend\n\nmodule {name}\n  def label\n    \"{l} ordinary\"\n  end\nend\n", l = x.to_lowercase()),
    )
}

/// The later opening's ShadowC runs its hook last, so its def stands, the
/// only one on the class: an earlier opening's copy is not the class's own
/// (#2278).
#[test]
fn a_later_openings_hook_replaces_an_earlier_openings_copy() {
    let shadows: Vec<(String, String)> = ["A", "B", "C"].iter().map(|x| shadow(x)).collect();
    let concerns: Vec<(&str, &str)> = shadows.iter().map(|(f, s)| (f.as_str(), s.as_str())).collect();
    let libs = [
        ("shadow_parent", "class ShadowParent < ApplicationController\n  include ShadowA\n  include ShadowB\nend\n"),
        ("shadow_parent_again", "class ShadowParent < ApplicationController\n  include ShadowC\nend\n"),
    ];
    let mut app = emit_and_run::empty_app()
        .write("app/controllers/application_controller.rb", "class ApplicationController < ActionController::API\nend\n");
    for (name, source) in &concerns {
        app = app.write(&format!("app/controllers/concerns/{name}.rb"), source);
    }
    for (name, source) in &libs {
        app = app.write(&format!("lib/{name}.rb"), source);
    }
    let (emitted, errors) = app.emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    let mut bodies = Vec::new();
    for file in ["shadow_parent", "shadow_parent_again"] {
        let Ok(class) = std::fs::read_to_string(emitted.join(format!("app/models/{file}.rb"))) else { continue };
        let lines: Vec<&str> = class.lines().filter(|l| !l.starts_with("#<SPINEL_SOURCE>")).map(str::trim).collect();
        bodies.extend((0..lines.len()).filter(|&i| lines[i] == "def label").map(|i| lines[i + 1].to_string()));
    }
    assert_eq!(bodies, ["\"c hook\""]);
    lib_case(&concerns, &libs, "ShadowParent", "\"c hook\"");
}

const PREP_M: (&str, &str) = ("prep_m", "module PrepM\n  extend ActiveSupport::Concern\n\n  def label\n    \"m\"\n  end\nend\n");
const PREP_N: (&str, &str) = ("prep_n", "module PrepN\n  extend ActiveSupport::Concern\n\n  def label\n    \"n\"\n  end\nend\n");
const PLAIN_M: (&str, &str) = ("plain_m", "module PlainM\n  def label\n    \"plain m\"\n  end\nend\n");

/// SingleController under PrepParentController (`parent`), with `child_lines`
/// and `concerns`: Ruby's `label` through the Ruby emit, and the body of the
/// `label` the Spinel emit splices into SingleController.
fn parent_case(concerns: &[(&str, &str)], parent: &str, child_lines: &str, want: &str) -> String {
    let app = || {
        let mut app = emit_and_run::empty_app()
            .write("app/controllers/application_controller.rb", "class ApplicationController < ActionController::API\nend\n")
            .write("app/controllers/prep_parent_controller.rb", &format!("class PrepParentController < ApplicationController\n{parent}end\n"));
        for (file, source) in concerns {
            app = app.write(&format!("app/controllers/concerns/{file}.rb"), source);
        }
        app.write(
            "app/controllers/single_controller.rb",
            &format!("class SingleController < PrepParentController\n{child_lines}\n  def show\n    render json: {{ label: label }}\n  end\nend\n"),
        )
        .write("config/routes.rb", "Rails.application.routes.draw do\n  get \"/single\" => \"single#show\"\nend\n")
    };
    let run = app().run_ruby(&format!(
        r##"status, _headers, body = begin
  Main.run_rack("REQUEST_METHOD" => "GET", "PATH_INFO" => "/single", "QUERY_STRING" => "", "rack.input" => StringIO.new(""))
rescue StandardError => e
  [e.class.name, {{}}, [e.message[0, 100]]]
end
text = +""
body.each {{ |part| text << part }}
got = [status, text]
want = [200, %({{"label":{want:?}}})]
raise "got #{{got.inspect}}, want #{{want.inspect}}" unless got == want
puts "parent case passed"
"##
    ));
    run.assert_passes();
    assert!(run.stdout.contains("parent case passed"), "{}", run.stdout);
    let (emitted, errors) = app().emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    let controller = std::fs::read_to_string(emitted.join("app/controllers/single_controller.rb")).unwrap();
    let at = controller.find("  def label\n").unwrap_or_else(|| panic!("no spliced label:\n{controller}"));
    let body = &controller[at + "  def label\n".len()..];
    body[..body.find("\n  end\n").unwrap()].trim().to_string()
}

/// The superclass's `prepend PrepM` puts PrepM ahead of it, its `include
/// PrepN` below: `[Single, PrepM, PrepParent, PrepN]`. The child's includes
/// of both add nothing (#2278).
#[test]
fn a_superclass_prepend_stays_ahead_of_its_includes() {
    let label = parent_case(&[PREP_M, PREP_N], "  prepend PrepM\n  include PrepN\n", "  include PrepN\n  include PrepM\n", "m");
    assert_eq!(label, "\"m\"");
}

/// ActiveSupport::Concern's `prepend_features` does nothing when the class
/// has the concern through its superclass: `[Single, PrepN, PrepParent,
/// PrepM]`. A plain module is prepended again: `[PlainM, Single, PrepN,
/// PrepParent, PlainM]` (#2278).
#[test]
fn a_concern_the_superclass_has_is_not_prepended_again() {
    let label = parent_case(&[PREP_M, PREP_N], "  include PrepM\n", "  prepend PrepM\n  include PrepN\n", "n");
    assert_eq!(label, "\"n\"");
    let label = parent_case(&[PLAIN_M, PREP_N], "  include PlainM\n", "  prepend PlainM\n  include PrepN\n", "plain m");
    assert_eq!(label, "\"plain m\"");
    // HookM's hook ran on the superclass and defined `label` there, below
    // the child's PrepN: `[Single, PrepN, PrepParent, HookM]`.
    let hook_m = ("hook_m", "module HookM\n  extend ActiveSupport::Concern\n\n  included do\n    def label\n      \"hook\"\n    end\n  end\nend\n");
    let label = parent_case(&[hook_m, PREP_N], "  include HookM\n", "  prepend HookM\n  include PrepN\n", "n");
    assert_eq!(label, "\"n\"");
}

/// A prepend runs the concern's `prepended do`, not its `included do`, so
/// HookDef defines no `label`, and DelegateOne delegates none:
/// `[HookDef, Single, PlainDef]` (#2278).
#[test]
fn a_prepend_runs_no_included_hook() {
    let lines = "  prepend HookDef\n  include PlainDef\n";
    assert_eq!(spliced_label(&[HOOK_DEF, PLAIN_DEF], lines, BuildTarget::Spinel), "\"plain\"");
    single_case(&[HOOK_DEF, PLAIN_DEF], lines, "label", "plain");
    let delegate_one = ("delegate_one", "module DelegateOne\n  extend ActiveSupport::Concern\n\n  included do\n    delegate :label, to: :source_one\n  end\nend\n");
    let lines = "  prepend DelegateOne\n  include PlainDef\n";
    single_case(&[delegate_one, PLAIN_DEF], lines, "label", "plain");
}

/// ShadowR's hook def is shadowed by its module's own `label`, but a hook
/// that runs later declares `attr_reader :label`: that reader replaces the
/// def on the includer, so Ruby gives nil. Run the other way round, the def
/// replaces the reader (#2278).
#[test]
fn a_later_hooks_reader_stands_over_a_shadowed_hook_def() {
    let shadow_r = ("shadow_r", "module ShadowR\n  extend ActiveSupport::Concern\n\n  included do\n    attr_reader :label\n    def label\n      \"hook\"\n    end\n  end\nend\n\nmodule ShadowR\n  def label\n    \"ordinary\"\n  end\nend\n");
    for (includes, body, want) in [
        ("  include ShadowR\n  include ReaderOnlyB\n", "@label", "null"),
        ("  include ReaderOnlyB\n  include ShadowR\n", "\"hook\"", "\"hook\""),
    ] {
        let libs = [("shadow_reader_parent", format!("class ShadowReaderParent < ApplicationController\n{includes}end\n"))];
        let libs: Vec<(&str, &str)> = libs.iter().map(|(f, s)| (*f, s.as_str())).collect();
        assert_eq!(lib_label_bodies(&[shadow_r, READER_ONLY_B], &libs, "shadow_reader_parent"), [body], "{includes:?}");
        lib_case(&[shadow_r, READER_ONLY_B], &libs, "ShadowReaderParent", want);
    }
}

/// The later opening's LastHookOnly runs its hook after the earlier
/// opening's ShadowB, so it wins although it needs no copy: ShadowB's copy
/// on the earlier opening goes (#2278).
#[test]
fn a_later_openings_unshadowed_hook_retires_an_earlier_copy() {
    let shadows: Vec<(String, String)> = ["A", "B"].iter().map(|x| shadow(x)).collect();
    let mut concerns: Vec<(&str, &str)> = shadows.iter().map(|(f, s)| (f.as_str(), s.as_str())).collect();
    concerns.push(LAST_HOOK_ONLY);
    let libs = [
        ("retire_parent", "class RetireParent < ApplicationController\n  include ShadowA\n  include ShadowB\nend\n"),
        ("retire_parent_again", "class RetireParent < ApplicationController\n  include LastHookOnly\nend\n"),
    ];
    let mut app = emit_and_run::empty_app()
        .write("app/controllers/application_controller.rb", "class ApplicationController < ActionController::API\nend\n");
    for (name, source) in &concerns {
        app = app.write(&format!("app/controllers/concerns/{name}.rb"), source);
    }
    for (name, source) in &libs {
        app = app.write(&format!("lib/{name}.rb"), source);
    }
    let (emitted, errors) = app.emit(BuildTarget::Spinel);
    assert!(errors.is_empty(), "{errors:#?}");
    let mut bodies = Vec::new();
    for (file, _) in &libs {
        let Ok(class) = std::fs::read_to_string(emitted.join(format!("app/models/{file}.rb"))) else { continue };
        let lines: Vec<&str> = class.lines().filter(|l| !l.starts_with("#<SPINEL_SOURCE>")).map(str::trim).collect();
        bodies.extend((0..lines.len()).filter(|&i| lines[i] == "def label").map(|i| lines[i + 1].to_string()));
    }
    assert!(!bodies.contains(&"\"b hook\"".to_string()), "{bodies:?}");
    lib_case(&concerns, &libs, "RetireParent", "\"last\"");
}

/// A prepend runs the concern's `prepended do`, not its `included do`, so
/// no filter its `included do` declares is registered: `@ran` stays unset
/// (#2278).
#[test]
fn a_prepend_registers_no_included_filter() {
    let filter = ("hook_filter", "module HookFilter\n  extend ActiveSupport::Concern\n\n  included do\n    before_action :from_hook\n  end\n\n  def from_hook\n    @ran = \"ran\"\n  end\nend\n");
    single_case(&[filter], "  prepend HookFilter\n", "(@ran || \"none\")", "none");
    let around = ("hook_around", "module HookAround\n  extend ActiveSupport::Concern\n\n  included do\n    around_action :wrap\n  end\n\n  def wrap\n    @ran = \"around\"\n    yield\n  end\nend\n");
    single_case(&[around], "  prepend HookAround\n", "(@ran || \"none\")", "none");
    // Nothing observes an `after_action` in the response: the emitted
    // controller registers none.
    let after = ("hook_after", "module HookAfter\n  extend ActiveSupport::Concern\n\n  included do\n    after_action :after_hook\n  end\n\n  def after_hook\n    nil\n  end\nend\n");
    let (emitted, errors) = single_app(&[after], "  prepend HookAfter\n", "(@ran || \"none\")").emit(BuildTarget::Ruby);
    assert!(errors.is_empty(), "{errors:#?}");
    let controller = std::fs::read_to_string(emitted.join("app/controllers/single_controller.rb")).unwrap();
    assert!(!controller.contains("self.after_hook"), "{controller}");
}

const LOOP_HOOK_HOST: (&str, &str) = (
    "loop_hook_host",
    "module LoopHookHost\n  extend ActiveSupport::Concern\n  %w[x].each { |m| class_eval \"included do\\n  def label\\n    \\\"hooked\\\"\\n  end\\nend\\n\" }\nend\n",
);

/// A `class_eval` loop's string runs in the concern's body, so its
/// `included do` is the concern's hook: its def is the includer's (#2278).
#[test]
fn a_class_eval_loops_included_hook_defines_on_the_includer() {
    single_case(&[LOOP_HOOK_HOST], "  include LoopHookHost\n", "label", "hooked");
    assert!(emitted_controller(&[LOOP_HOOK_HOST], "  include LoopHookHost\n", BuildTarget::Spinel).contains("\"hooked\""));
}

/// A `def` the string writes beside its `included do` is the concern's own,
/// not the hook's: PlainLabel, included later, stands ahead of it (#2278).
#[test]
fn a_def_beside_a_loops_included_hook_stays_the_concerns() {
    let host = (
        "loop_hook_plain_beside",
        "module LoopHookPlainBeside\n  extend ActiveSupport::Concern\n  %w[x].each { |m| class_eval \"def label\\n  \\\"plain\\\"\\nend\\nincluded do\\n  def hooked\\n    \\\"hooked\\\"\\n  end\\nend\\n\" }\nend\n",
    );
    let plain = ("plain_label", "module PlainLabel\n  extend ActiveSupport::Concern\n\n  def label\n    \"other\"\n  end\nend\n");
    let lines = "  include LoopHookPlainBeside\n  include PlainLabel\n";
    single_case(&[host, plain], lines, "label", "other");
    single_case(&[host, plain], lines, "hooked", "hooked");
}

/// A hook def of a name the concern's body also defines: Ruby gives the
/// hook's (`"hooked"`, defined on the includer), which the splice doesn't
/// model, so it is refused as the same `included do` written in the body
/// is, not answered with the body's def (#2278).
#[test]
fn a_loops_hook_def_of_the_hosts_name_is_refused_as_the_bodys_is() {
    let hook = "included do\\n  def label\\n    \\\"hooked\\\"\\n  end\\nend\\n";
    let looped = ("loop_hook_own_def", format!("module LoopHookOwnDef\n  extend ActiveSupport::Concern\n  %w[x].each {{ |m| class_eval \"{hook}\" }}\n\n  def label\n    \"host\"\n  end\nend\n"));
    let direct = ("direct_hook_own_def", "module DirectHookOwnDef\n  extend ActiveSupport::Concern\n  included do\n    def label\n      \"hooked\"\n    end\n  end\n\n  def label\n    \"host\"\n  end\nend\n");
    let refusal = |files: &[(&str, &str)], lines: &str| {
        let (_, errors) = single_app(files, lines, "label").emit(BuildTarget::Ruby);
        errors.iter().map(|e| format!("{e:?}")).find(|e| e.contains("nested def not supported")).map(|e| e.contains("`def label`"))
    };
    assert_eq!(refusal(&[direct], "  include DirectHookOwnDef\n"), Some(true));
    assert_eq!(refusal(&[(looped.0, looped.1.as_str())], "  include LoopHookOwnDef\n"), Some(true));
}

/// Without `extend ActiveSupport::Concern`, `included do` is Module#included
/// called with no argument, which raises as the module loads: the string's
/// hook is not read as a concern's, and nothing of it reaches the includer.
#[test]
fn a_plain_modules_loop_included_block_is_no_hook() {
    let host = ("loop_not_concern", "module LoopNotConcern\n  %w[x].each { |m| class_eval \"included do\\n  def label\\n    \\\"hooked\\\"\\n  end\\nend\\n\" }\nend\n");
    for target in [BuildTarget::Ruby, BuildTarget::Spinel] {
        assert!(!emitted_controller(&[host], "  include LoopNotConcern\n", target).contains("hooked"), "{target:?}");
    }
}

/// A module whose body only extends ActiveSupport::Concern, with no loop,
/// stays out of the library classes as before: only a `class_eval` loop
/// that runs in the concern keeps a module with no `def` of its own (#2278).
#[test]
fn a_concern_with_only_its_extend_is_still_not_a_library_class() {
    let only_extend = "module OnlyExtend\n  extend ActiveSupport::Concern\nend\n";
    assert!(roundhouse::ingest::ingest_library_classes(only_extend.as_bytes(), "only_extend.rb").expect("ingest").is_empty());
    let looped = "module LoopOnly\n  extend ActiveSupport::Concern\n  %w[x].each { |m| class_eval \"included do\\n  def label\\n    \\\"hooked\\\"\\n  end\\nend\\n\" }\nend\n";
    let classes = roundhouse::ingest::ingest_library_classes(looped.as_bytes(), "loop_only.rb").expect("ingest");
    assert_eq!(classes.iter().map(|c| c.name.0.as_str()).collect::<Vec<_>>(), ["LoopOnly"]);
}

/// A concern's `class_eval` loop string with the body's `value` in it.
fn loop_aliased(file: &str, module: &str, string: &str) -> (String, String) {
    (file.to_string(), format!("module {module}\n  extend ActiveSupport::Concern\n\n  def value\n    \"real\"\n  end\n\n  %w[x].each {{ |m| class_eval \"{string}\" }}\nend\n"))
}

/// `alias_method` in a concern's `class_eval` loop string copies the body's
/// own `def`, written before the loop: the includer's `label` is `value`'s
/// (#2278). The wrapped string doesn't define `value`, so it is not answered
/// with an empty method of the name either.
#[test]
fn a_loops_alias_method_of_the_concerns_def_copies_it() {
    let (file, source) = loop_aliased("alias_method_host", "AliasMethodHost", "alias_method :label, :value");
    single_case(&[(file.as_str(), source.as_str())], "  include AliasMethodHost\n", "label", "real");
}

/// The keyword `alias` in a loop string is still refused, as before the
/// string ran in the concern (#2278).
#[test]
fn a_loops_keyword_alias_of_the_concerns_def_is_refused() {
    let (file, source) = loop_aliased("alias_keyword_host", "AliasKeywordHost", "alias label value");
    let mut tree: std::collections::HashMap<std::path::PathBuf, Vec<u8>> = std::collections::HashMap::new();
    tree.insert(format!("app/controllers/concerns/{file}.rb").into(), source.into_bytes());
    let refused = roundhouse::ingest::ingest_app_from_tree(tree).map(|_| ()).expect_err("the keyword alias is refused");
    assert!(format!("{refused:?}").contains("alias names a method this body has not defined"), "{refused:?}");
}

/// The `label` of each module a class_eval loop test includes.
fn labelled(file: &str, module: &str, label: &str) -> (String, String) {
    (file.to_string(), format!("module {module}\n  def label\n    \"{label}\"\n  end\nend\n"))
}

/// A loop's elements run in order, each string's `include` after the
/// previous one's: `[Single, LoopTwoHost, LoopTwoY, LoopTwoX]`, so `label`
/// is LoopTwoY's (#2278).
#[test]
fn a_class_eval_loops_elements_include_in_order() {
    let host = ("loop_two_host", "module LoopTwoHost\n  def loop_two_host_own\n    \"own\"\n  end\n\n  %w[LoopTwoX LoopTwoY].each { |m| class_eval \"include #{m}\" }\nend\n");
    let x = labelled("loop_two_x", "LoopTwoX", "loop x");
    let y = labelled("loop_two_y", "LoopTwoY", "loop y");
    single_case(&[(x.0.as_str(), x.1.as_str()), (y.0.as_str(), y.1.as_str()), host], "  include LoopTwoHost\n", "label", "loop y");
}

/// One string's `include` statements, `;`-separated, run in order: the
/// last, the host's nested `Leaf`, is nearest (#2278).
#[test]
fn a_class_eval_loop_strings_include_statements_run_in_order() {
    let host = (
        "loop_five_host",
        "module LoopFiveHost\n  def loop_five_host_own\n    \"own\"\n  end\n\n  module Leaf\n    def label\n      \"five leaf\"\n    end\n  end\n\n  %w[x].each { |m| class_eval \"include LoopFiveA; include LoopFiveB; include LoopFiveC; include LoopFiveD; include Leaf\" }\nend\n",
    );
    let mods: Vec<(String, String)> = ["a", "b", "c", "d"]
        .iter()
        .map(|n| labelled(&format!("loop_five_{n}"), &format!("LoopFive{}", n.to_uppercase()), &format!("five {n}")))
        .collect();
    let mut files: Vec<(&str, &str)> = mods.iter().map(|(f, s)| (f.as_str(), s.as_str())).collect();
    files.push(host);
    single_case(&files, "  include LoopFiveHost\n", "label", "five leaf");
}

/// Two `include` lines in one element's string are two statements, run in
/// order: LoopLinesB is nearest (#2278).
#[test]
fn a_class_eval_loop_strings_include_lines_are_two_statements() {
    let host = ("loop_lines_host", "module LoopLinesHost\n  def loop_lines_host_own\n    \"own\"\n  end\n\n  %w[x].each { |m| class_eval \"include LoopLinesA\\ninclude LoopLinesB\" }\nend\n");
    let a = labelled("loop_lines_a", "LoopLinesA", "lines a");
    let b = labelled("loop_lines_b", "LoopLinesB", "lines b");
    single_case(&[(a.0.as_str(), a.1.as_str()), (b.0.as_str(), b.1.as_str()), host], "  include LoopLinesHost\n", "label", "lines b");
}

/// `include A, B` in a loop's string stays one statement: Ruby includes
/// its arguments last first, so LoopCommaA is nearest (#2278).
#[test]
fn a_class_eval_loop_strings_include_of_two_modules_stays_one_statement() {
    let host = ("loop_comma_host", "module LoopCommaHost\n  def loop_comma_host_own\n    \"own\"\n  end\n\n  %w[x].each { |m| class_eval \"include LoopCommaA, LoopCommaB\" }\nend\n");
    let a = labelled("loop_comma_a", "LoopCommaA", "comma a");
    let b = labelled("loop_comma_b", "LoopCommaB", "comma b");
    single_case(&[(a.0.as_str(), a.1.as_str()), (b.0.as_str(), b.1.as_str()), host], "  include LoopCommaHost\n", "label", "comma a");
}

/// The Ruby emit writes a loop's two `include` statements as two lines in
/// the order they ran, not as one `include LoopTwoX, LoopTwoY`, which
/// Ruby would run last first (#2278).
#[test]
fn the_ruby_emit_writes_a_loops_include_statements_as_two_lines() {
    let host = ("loop_two_host", "module LoopTwoHost\n  def loop_two_host_own\n    \"own\"\n  end\n\n  %w[LoopTwoX LoopTwoY].each { |m| class_eval \"include #{m}\" }\nend\n");
    let x = labelled("loop_two_x", "LoopTwoX", "loop x");
    let y = labelled("loop_two_y", "LoopTwoY", "loop y");
    let (emitted, errors) = single_app(&[(x.0.as_str(), x.1.as_str()), (y.0.as_str(), y.1.as_str()), host], "  include LoopTwoHost\n", "label").emit(BuildTarget::Ruby);
    assert!(errors.is_empty(), "{errors:#?}");
    let path = emitted.join("app/models/loop_two_host.rb");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(text.contains("  include LoopTwoX\n  include LoopTwoY\n"), "{text}");
}

/// A loop host whose includes are filtered afterwards without their spans
/// (`included_extend` drops the include of a hook that only extends its
/// includer): the class reads its include statements as main does, one per
/// module, not from a record out of step with them (#2278).
#[test]
fn a_loop_hosts_include_filtered_alone_falls_back_to_the_span_reading() {
    let host = (
        "loop_filter_host",
        "module LoopFilterHost\n  def loop_filter_host_own\n    \"own\"\n  end\n\n  %w[LoopFilterHook LoopFilterX LoopFilterY].each { |m| class_eval \"include #{m}\" }\nend\n",
    );
    let hook = ("loop_filter_hook", "module LoopFilterHook\n  def self.included(base)\n    base.instance_eval { extend LoopFilterMethods }\n  end\nend\n");
    let methods = ("loop_filter_methods", "module LoopFilterMethods\n  def greeting\n    \"hi\"\n  end\nend\n");
    let x = labelled("loop_filter_x", "LoopFilterX", "filter x");
    let y = labelled("loop_filter_y", "LoopFilterY", "filter y");
    let files = [hook, methods, (x.0.as_str(), x.1.as_str()), (y.0.as_str(), y.1.as_str()), host];
    let mut tree: std::collections::HashMap<std::path::PathBuf, Vec<u8>> = std::collections::HashMap::new();
    for (file, source) in &files {
        tree.insert(format!("app/controllers/concerns/{file}.rb").into(), source.as_bytes().to_vec());
    }
    let app = roundhouse::ingest::ingest_app_from_tree(tree).expect("ingest");
    let host_class = app.library_classes.iter().find(|c| c.name.0.as_str() == "LoopFilterHost").expect("LoopFilterHost");
    let includes: Vec<&str> = host_class.includes.iter().map(|i| i.0.as_str()).collect();
    assert_eq!(includes, ["LoopFilterX", "LoopFilterY"], "the hook's include is filtered out");
    single_case(&files, "  include LoopFilterHost\n", "label", "filter y");
}


/// An `alias_method` in a loop string naming a `def` the body writes after
/// the loop gives the includer no `label`, as before. Ruby raises NameError
/// at the loop, since `later` isn't defined yet (#2278).
#[test]
fn a_loops_alias_method_of_a_later_def_copies_nothing() {
    let source = "module AliasLaterHost\n  extend ActiveSupport::Concern\n\n  %w[x].each { |m| class_eval \"alias_method :label, :later\" }\n\n  def later\n    \"late\"\n  end\nend\n";
    let controller = emitted_controller(&[("alias_later_host", source)], "  include AliasLaterHost\n", BuildTarget::Ruby);
    assert!(!controller.contains("def label"), "{controller}");
}

/// The same `alias_method` in a plain module's loop string copies its `def`
/// (#2278).
#[test]
fn a_plain_modules_loop_alias_method_copies_its_def() {
    let source = "module AliasPlainHost\n  def value\n    \"plain real\"\n  end\n\n  %w[x].each { |m| class_eval \"alias_method :label, :value\" }\nend\n";
    single_case(&[("alias_plain_host", source)], "  include AliasPlainHost\n", "label", "plain real");
}

/// A concern's loop string aliasing a `def` only its `included do` writes
/// copies nothing: that `def` is the includer's, not the concern's, and Ruby
/// raises NameError at the loop (#2278).
#[test]
fn a_loops_alias_method_of_a_hook_def_copies_nothing() {
    let source = "module AliasHookOnlyHost\n  extend ActiveSupport::Concern\n\n  included do\n    def hooked\n      \"hook\"\n    end\n  end\n\n  %w[x].each { |m| class_eval \"alias_method :label, :hooked\" }\nend\n";
    let controller = emitted_controller(&[("alias_hook_only_host", source)], "  include AliasHookOnlyHost\n", BuildTarget::Ruby);
    assert!(!controller.contains("def label"), "{controller}");
}
