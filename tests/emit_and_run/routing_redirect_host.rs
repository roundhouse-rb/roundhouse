use super::emit_and_run;

fn app() -> emit_and_run::Overlay {
    emit_and_run::real_blog()
        .edit(
            "config/routes.rb",
            "  root \"articles#index\"\n",
            r##"  root "articles#index"
  get "/outside", to: redirect("https://other.test/landing")
  get "/moved/:id", to: redirect(status: 302) { |params, req| "#{req.protocol}other.test/items/#{params[:id]}" }
  get "/relative", to: redirect("/articles")
  get "/controller-outside", to: "outbound#show"
  get "/scheme-no-slash", to: "outbound#scheme_no_slash"
  get "/scheme-one-slash", to: "outbound#scheme_one_slash"
  get "/scheme-opaque", to: "outbound#scheme_opaque"
  get "/bare-relative", to: "outbound#bare_relative"
  get "/rooted", to: "outbound#rooted"
"##,
        )
        .write(
            "app/controllers/outbound_controller.rb",
            r#"class OutboundController < ApplicationController
  def show
    redirect_to "https://other.test/landing"
  end

  # A scheme written without `//` has no URI authority, so the
  # same-host check has no host to compare — but a browser navigates
  # it to `https://evil.test/` all the same.
  def scheme_no_slash
    redirect_to "https:evil.test"
  end

  def scheme_one_slash
    redirect_to "https:/evil.test"
  end

  def scheme_opaque
    redirect_to "javascript:alert(1)"
  end

  def bare_relative
    redirect_to "evil.test"
  end

  def rooted
    redirect_to "/articles"
  end
end
"#,
        )
}

#[test]
fn routing_redirects_allow_other_hosts_and_controllers_still_refuse_them() {
    app()
        .run_ruby(
            r#"
[
  ["/outside", 301, "https://other.test/landing"],
  ["/moved/7", 302, "http://other.test/items/7"],
  ["/relative", 301, "/articles"]
].each do |path, expected_status, expected_location|
  status, headers, body = Main.run_rack(
    "REQUEST_METHOD" => "GET", "PATH_INFO" => path, "QUERY_STRING" => "",
    "HTTP_HOST" => "app.test", "rack.input" => StringIO.new("")
  )
  raise "status: #{status}" unless status == expected_status
  raise "location: #{headers['location']}" unless headers["location"] == expected_location
  raise "redirect body" unless body.join.include?("Redirecting")
end
refused = false
begin
  Main.run_rack(
    "REQUEST_METHOD" => "GET", "PATH_INFO" => "/controller-outside", "QUERY_STRING" => "",
    "HTTP_HOST" => "app.test", "rack.input" => StringIO.new("")
  )
rescue ArgumentError => error
  refused = error.message.include?("Unsafe redirect")
end
raise "controller allowed another host" unless refused
puts "routing redirects allow other hosts; controller redirects refuse them"
"#,
        )
        .assert_passes();
}

#[test]
#[ignore = "requires the Spinel toolchain"]
fn routing_redirect_host_policy_runs_in_emitted_spinel() {
    app()
        .run_spinel_with_rbs(
            r#"
controller = RoundhouseRedirectsController.new
controller.outside
raise "status" unless controller.status == 301
raise "location" unless controller.location == "https://other.test/landing"
refused = false
begin
  ordinary = ActionController::Base.new
  ordinary.redirect_to("https://other.test/landing")
rescue ArgumentError
  refused = true
end
raise "ordinary controller allowed another host" unless refused
puts "routing host policy passed natively"
"#,
        )
        .assert_passes();
}

/// A scheme with no `//` is an open redirect unless the same-host check
/// refuses it. `ActionController.location_host` reports a host only for
/// `//host` and `scheme://host`, so `https:evil.test` and
/// `https:/evil.test` reach `same_host_location` with no host at all —
/// and before the hostless branch required a single-slash path, they
/// were classified "relative" and sent as the `Location`. Browsers
/// apply WHATWG special-scheme parsing and navigate `https:evil.test`
/// to `https://evil.test/`, so `redirect_to params[:back]` was an open
/// redirect for every app.
///
/// Rails 8.1 refuses all four of these: `URI(url).host` is nil and the
/// url does not start with `/`, so `_url_host_allowed?` is false and
/// `redirect_to` raises. A single-slash path is the one hostless
/// location it allows, and it still does here.
#[test]
fn a_scheme_without_slashes_is_refused_like_any_other_host() {
    app()
        .run_ruby(
            r#"
["/scheme-no-slash", "/scheme-one-slash", "/scheme-opaque", "/bare-relative"].each do |path|
  refused = nil
  begin
    Main.run_rack(
      "REQUEST_METHOD" => "GET", "PATH_INFO" => path, "QUERY_STRING" => "",
      "HTTP_HOST" => "app.test", "rack.input" => StringIO.new("")
    )
  rescue ArgumentError => error
    refused = error.message
  end
  raise "not refused: " + path if refused.nil?
  raise "wrong refusal for " + path + ": " + refused unless refused.include?("Unsafe redirect")
end

# The one hostless location Rails allows still goes through, absolute
# against the request's own host as `_compute_redirect_to_location` makes it.
status, headers, _body = Main.run_rack(
  "REQUEST_METHOD" => "GET", "PATH_INFO" => "/rooted", "QUERY_STRING" => "",
  "HTTP_HOST" => "app.test", "rack.input" => StringIO.new("")
)
raise "rooted status: " + status.to_s unless status == 302
raise "rooted location: " + headers["location"].to_s unless headers["location"] == "http://app.test/articles"
puts "a scheme without slashes is refused; a rooted path still redirects"
"#,
        )
        .assert_passes();
}
