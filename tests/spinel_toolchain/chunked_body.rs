use super::emit_and_run;
use std::path::Path;
use std::process::Command;

#[test]
#[ignore = "requires the Spinel toolchain, run in its CI lane"]
fn generated_controller_receives_decoded_params_and_headers() {
    let tree = emit_and_run::real_blog()
        .edit(
            "config/routes.rb",
            "  root \"articles#index\"\n",
            "  root \"articles#index\"\n  post '/frames', to: 'frames#echo'\n",
        )
        .write(
            "app/controllers/frames_controller.rb",
            r#"
class FramesController < ActionController::API
  def echo
    render plain: params[:title].to_s + "|" + request.headers["Content-Length"].to_s +
      "|" + request.headers["Transfer-Encoding"].to_s
  end
end
"#,
        )
        .native_http_tree();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("ruby")
        .arg(root.join("tests/spinel_chunked_body_app_check.rb"))
        .arg(tree.join("build/bin/blog"))
        .current_dir(&tree)
        .output()
        .expect("drive generated controller");
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
