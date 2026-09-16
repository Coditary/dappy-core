use dap_plugin_api::{PluginError, default_builtin_dir, load_from_file};

#[test]
fn builtin_python_manifest_resolves_attachment_path() {
    let path = default_builtin_dir().join("python").join("plugin.yaml");
    let manifest = load_from_file(&path).expect("python manifest");
    assert_eq!(manifest.attachment.as_deref(), Some("attachment.yaml"));
    let attachment = manifest.attachment_path().expect("attachment path");
    assert!(attachment.is_file());
    assert!(attachment.ends_with("python/attachment.yaml"));
}

#[test]
fn rejects_non_yaml_attachment_extension() {
    let dir = std::env::temp_dir().join(format!(
        "dap-plugin-attachment-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let manifest_path = dir.join("demo.yaml");
    std::fs::write(
        &manifest_path,
        r#"
id: demo
name: Demo
version: 0.1.0
launchTypes: [demo]
attachment: demo.json
adapter:
  transport: stdio
  command: demo
"#,
    )
    .expect("write manifest");

    let err = load_from_file(&manifest_path).unwrap_err();
    assert!(matches!(err, PluginError::InvalidManifest(_)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn skips_attachment_yaml_when_scanning_plugin_dir() {
    let manifests =
        dap_plugin_api::load_from_dir(&default_builtin_dir()).expect("load builtin plugins");
    assert!(manifests.iter().any(|m| m.id == "python"));
    assert!(!manifests.iter().any(|m| m.id == "python.attachment"));
}
