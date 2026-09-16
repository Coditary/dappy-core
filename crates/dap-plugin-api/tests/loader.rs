use std::path::PathBuf;

use dap_plugin_api::{PluginError, default_builtin_dir, load_from_dir, load_from_file};

#[test]
fn loads_manifest_from_per_plugin_directory() {
    let dir = std::env::temp_dir().join(format!(
        "dap-plugin-dir-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let plugin_dir = dir.join("demo");
    std::fs::create_dir_all(&plugin_dir).expect("tmpdir");
    std::fs::write(
        plugin_dir.join("plugin.yaml"),
        r#"
id: demo
name: Demo
version: 0.1.0
launchTypes: [demo]
adapter:
  transport: stdio
  command: demo
"#,
    )
    .expect("write");
    std::fs::write(
        plugin_dir.join("attachment.yaml"),
        "version: 1\n",
    )
    .expect("write attachment");

    let manifests = load_from_dir(&dir).expect("load");
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].id, "demo");
    let _ = std::fs::remove_dir_all(dir);
}

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
