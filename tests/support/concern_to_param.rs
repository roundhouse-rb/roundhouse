//! #1897: a model whose `to_param` comes from an included concern. Rails
//! fills a route helper's segment from it, and a bare `to_param` call
//! answers it. `MODULE` defines it as a method of the module, `INCLUDED`
//! inside the concern's `included do`, which Rails runs on the includer.
//! Each line is tagged `L<n>:`, and `RENDERS` is what Rails 8.1.4 renders
//! for both. The ruby target and Spinel check each line's whole output.

use super::emit_and_run::Overlay;

pub const VIEW: &str = concat!(
    "<% rec = @article %>\n",
    r#"L0:<%= "" + article_path(rec) %>"#, "\n",
    r#"L1:<%= link_to "x", article_path(rec) %>"#, "\n",
    "L2:<%= rec.to_param %>\n",
);

pub const RENDERS: &[&str] = &["L0:/articles/s-7", r#"L1:<a href="/articles/s-7">x</a>"#, "L2:s-7"];

pub const MODULE: &str = "module Sluggable\n  def to_param\n    \"s-#{id}\"\n  end\nend\n";

pub const INCLUDED: &str =
    "module Sluggable\n  extend ActiveSupport::Concern\n\n  included do\n    def to_param\n      \"s-#{id}\"\n    end\n  end\nend\n";

/// `base` with the `@article` record, `concern` as `Sluggable` and
/// `Article` including it.
pub fn app(base: Overlay, concern: &str) -> Overlay {
    super::nested_route_helper_record::app(base, false)
        .write("app/models/concerns/sluggable.rb", concern)
        .edit("app/models/article.rb", "class Article < ApplicationRecord\n", "class Article < ApplicationRecord\n  include Sluggable\n")
}
