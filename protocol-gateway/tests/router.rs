use std::sync::Arc;

use protocol_gateway::{PluginRegistry, RouteContext, Router, SpawnSpec, StaticPlugin};

#[test]
fn routes_by_file_extension() {
    let mut registry = PluginRegistry::new();
    registry
        .register(Arc::new(StaticPlugin {
            id: "python".into(),
            name: "Python".into(),
            launch_types: vec!["python".into()],
            file_extensions: vec!["py".into()],
            spawn: SpawnSpec::stdio("python", vec!["-m".into(), "debugpy.adapter".into()]),
        }))
        .unwrap();

    let router = Router::new(&registry);
    let ctx = RouteContext::new().with_program_path("main.py");
    let matched = router.resolve(&ctx).unwrap();
    assert_eq!(matched.plugin_id, "python");
}

#[test]
fn explicit_plugin_overrides_extension() {
    let mut registry = PluginRegistry::new();
    registry
        .register(Arc::new(StaticPlugin {
            id: "python".into(),
            name: "Python".into(),
            launch_types: vec![],
            file_extensions: vec!["py".into()],
            spawn: SpawnSpec::stdio("python", vec![]),
        }))
        .unwrap();
    registry
        .register(Arc::new(StaticPlugin {
            id: "custom".into(),
            name: "Custom".into(),
            launch_types: vec![],
            file_extensions: vec![],
            spawn: SpawnSpec::stdio("custom", vec![]),
        }))
        .unwrap();

    let router = Router::new(&registry);
    let ctx = RouteContext::new()
        .with_program_path("main.py")
        .with_explicit_plugin("custom");
    let matched = router.resolve(&ctx).unwrap();
    assert_eq!(matched.plugin_id, "custom");
}
