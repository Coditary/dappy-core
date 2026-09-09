use dap_core::{DapEngine, LaunchOptions, manifest_to_static_plugin};
use dap_plugin_api::{AdapterSpawn, PluginManifest, SpawnTransport};

#[test]
fn manifest_to_static_plugin_round_trip() {
    let manifest = PluginManifest {
        id: "demo".into(),
        name: "Demo".into(),
        version: "1.0.0".into(),
        languages: vec!["demo".into()],
        launch_types: vec!["demo".into()],
        file_extensions: vec!["demo".into()],
        adapter: AdapterSpawn {
            transport: SpawnTransport::Stdio,
            command: "echo".into(),
            args: vec![],
        },
    };
    let plugin = manifest_to_static_plugin(&manifest);
    assert_eq!(plugin.id, "demo");
    match plugin.spawn {
        protocol_gateway::SpawnSpec::Stdio { command, .. } => assert_eq!(command, "echo"),
        _ => panic!("expected stdio spawn"),
    }
}

#[tokio::test]
async fn resolve_route_prefers_explicit_adapter() {
    let engine = DapEngine::with_builtin_plugins().expect("builtin plugins");
    let route = dap_core::resolve_route(
        &engine.registry,
        &LaunchOptions {
            request: "launch".into(),
            program: Some("main.py".into()),
            adapter: Some("fake".into()),
            extra: serde_json::json!({}),
        },
    )
    .expect("route");
    assert_eq!(route.adapter_id, "fake");
}

#[tokio::test]
async fn resolve_route_by_cargo_binary_without_adapter() {
    let engine = DapEngine::with_builtin_plugins().expect("builtin plugins");
    let route = dap_core::resolve_route(
        &engine.registry,
        &LaunchOptions {
            request: "launch".into(),
            program: Some("target/debug/myapp".into()),
            adapter: None,
            extra: serde_json::json!({}),
        },
    )
    .expect("route");
    assert_eq!(route.adapter_id, "rust");
}

#[test]
fn spawn_spec_to_adapter_round_trip() {
    use dap_core::spawn_spec_to_adapter;
    let spec = protocol_gateway::SpawnSpec::stdio("fake", vec!["--flag".into()]);
    let adapter = spawn_spec_to_adapter(&spec);
    assert_eq!(adapter.command, "fake");
    assert_eq!(adapter.args, vec!["--flag".to_string()]);

    let tcp = protocol_gateway::SpawnSpec::Tcp {
        host: "127.0.0.1".into(),
        port: 9000,
    };
    let tcp_adapter = spawn_spec_to_adapter(&tcp);
    assert_eq!(tcp_adapter.command, "127.0.0.1:9000");
    assert!(tcp_adapter.args.is_empty());
}
