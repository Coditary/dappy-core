use dap_core::{DapEngine, default_plugins_dir, load_plugins_from_dir};
use dap_plugin_api::{AdapterSpawn, SpawnTransport};

#[test]
fn loads_builtin_fake_plugin() {
    let dir = default_plugins_dir();
    let manifests = load_plugins_from_dir(&dir).expect("load plugins");
    assert!(
        manifests.iter().any(|m| m.id == "fake"),
        "expected fake plugin in {}",
        dir.display()
    );
    assert!(
        manifests.iter().any(|m| m.id == "rust"),
        "expected rust plugin in {}",
        dir.display()
    );
}

#[tokio::test]
async fn spawn_fake_adapter_if_available() {
    let adapter = std::env::var("CARGO_BIN_EXE_fake-dap-adapter").ok();
    if adapter.is_none() {
        return;
    }

    dap_core::Backend::spawn(&AdapterSpawn {
        transport: SpawnTransport::Stdio,
        command: adapter.unwrap(),
        args: vec![],
    })
    .await
    .expect("spawn fake adapter");
}

#[tokio::test]
async fn engine_loads_builtin_plugins() {
    let engine = DapEngine::with_builtin_plugins().expect("builtin plugins");
    assert!(engine.adapter_spawn("fake").is_some());
    assert!(engine.adapter_spawn("rust").is_some());
}

#[tokio::test]
async fn tcp_adapter_spawn_rejects_invalid_endpoint() {
    let err = dap_core::Backend::spawn(&AdapterSpawn {
        transport: SpawnTransport::Tcp,
        command: "not-an-endpoint".into(),
        args: vec![],
    })
    .await;
    assert!(err.is_err());
}

#[tokio::test]
async fn tcp_adapter_spawn_fails_for_unreachable_port() {
    let err = dap_core::Backend::spawn_tcp("127.0.0.1", 1).await;
    assert!(err.is_err());
}

#[test]
fn load_plugins_from_missing_dir_returns_empty() {
    let manifests =
        load_plugins_from_dir(std::path::Path::new("/nonexistent-dap-plugins-dir")).unwrap();
    assert!(manifests.is_empty());
}
