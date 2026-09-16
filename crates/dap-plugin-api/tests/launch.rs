use dap_plugin_api::{default_builtin_dir, load_from_file};

fn fixture_path(id: &str) -> std::path::PathBuf {
    default_builtin_dir().join(id).join("plugin.yaml")
}

#[test]
fn python_manifest_builds_debugpy_launch_payload() {
    let manifest = load_from_file(&fixture_path("python")).expect("python plugin");
    let args = manifest.build_launch_arguments("main.py");
    assert_eq!(args["type"], "debugpy");
    assert_eq!(args["request"], "launch");
    assert_eq!(args["stopOnEntry"], false);
    assert!(args.get("program").and_then(|v| v.as_str()).is_some());
}

#[test]
fn rust_manifest_includes_source_languages() {
    let manifest = load_from_file(&fixture_path("rust")).expect("rust plugin");
    let args = manifest.build_launch_arguments("target/debug/app");
    assert_eq!(args["sourceLanguages"], serde_json::json!(["rust"]));
}

#[test]
fn python_manifest_declares_custom_init_steps() {
    let manifest = load_from_file(&fixture_path("python")).expect("python plugin");
    assert_eq!(manifest.init_steps().len(), 4);
    assert_eq!(manifest.init_steps()[0].request.as_deref(), Some("launch"));
    assert!(manifest.init_steps()[0].use_launch_arguments);
}
