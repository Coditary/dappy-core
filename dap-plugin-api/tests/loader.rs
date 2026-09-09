use std::path::PathBuf;

use dap_plugin_api::{PluginError, default_builtin_dir, load_from_dir, load_from_file};

#[test]
fn loads_builtin_fake_plugin() {
    let dir = default_builtin_dir();
    let manifests = load_from_dir(&dir).expect("load builtin plugins");
    assert!(
        manifests.iter().any(|m| m.id == "fake"),
        "expected fake plugin in {}",
        dir.display()
    );
}

#[test]
fn missing_dir_returns_empty() {
    let manifests =
        load_from_dir(PathBuf::from("/nonexistent-dap-plugins-dir").as_path()).expect("empty");
    assert!(manifests.is_empty());
}

#[test]
fn rejects_invalid_manifest() {
    let dir = std::env::temp_dir().join(format!(
        "dap-plugin-invalid-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let path = dir.join("bad.yaml");
    std::fs::write(
        &path,
        r#"
id: ""
name: Bad
version: 0.1.0
launchTypes: [fake]
adapter:
  transport: stdio
  command: fake
"#,
    )
    .expect("write");

    let err = load_from_file(&path).unwrap_err();
    assert!(matches!(err, PluginError::InvalidManifest(_)));
    let _ = std::fs::remove_dir_all(dir);
}
