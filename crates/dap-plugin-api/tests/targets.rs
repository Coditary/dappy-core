use dap_plugin_api::{
    default_builtin_dir, load_target_defaults, load_target_from_file, target_manifest_in_dir,
};

#[test]
fn builtin_gdbserver_target_manifest_loads() {
    let dir = default_builtin_dir().join("gdbserver");
    let path = target_manifest_in_dir(&dir).expect("target manifest path");
    let manifest = load_target_from_file(&path).expect("gdbserver target");
    assert_eq!(manifest.id, "gdbserver");
    assert!(manifest.spawn.is_some());
}

#[test]
fn load_target_defaults_includes_builtin_entries() {
    let manifests = load_target_defaults().expect("target defaults");
    assert!(manifests.iter().any(|manifest| manifest.id == "gdbserver"));
    assert!(
        manifests
            .iter()
            .any(|manifest| manifest.id == "qemu-x86-kernel")
    );
    assert!(manifests.iter().any(|manifest| manifest.id == "rsp-attach"));
}
