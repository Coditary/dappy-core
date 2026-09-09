use dap_core::{DapEngine, LaunchOptions};
use dap_plugin_api::{AdapterSpawn, PluginManifest, SpawnTransport};

#[tokio::test]
async fn routes_python_by_extension() {
    let mut engine = DapEngine::new();
    engine.register_plugin(PluginManifest {
        id: "python".into(),
        name: "Python".into(),
        version: "0.1.0".into(),
        languages: vec!["python".into()],
        launch_types: vec!["python".into()],
        file_extensions: vec!["py".into()],
        adapter: AdapterSpawn {
            transport: SpawnTransport::Stdio,
            command: "python".into(),
            args: vec!["-m".into(), "debugpy.adapter".into()],
        },
    });

    let session = engine
        .prepare_session(LaunchOptions {
            request: "launch".into(),
            program: Some("main.py".into()),
            adapter: None,
            extra: serde_json::json!({}),
        })
        .await
        .unwrap();

    assert_eq!(session.adapter_id, "python");
}

#[tokio::test]
async fn routes_rust_by_cargo_binary_path() {
    let engine = DapEngine::with_builtin_plugins().expect("builtin plugins");

    let spawn = engine
        .resolve_adapter_spawn(&LaunchOptions {
            request: "launch".into(),
            program: Some("target/debug/myapp".into()),
            adapter: None,
            extra: serde_json::json!({}),
        })
        .expect("resolve rust");

    assert_eq!(spawn.command, "lldb-dap");
}

#[tokio::test]
async fn routes_rust_by_explicit_adapter() {
    let engine = DapEngine::with_builtin_plugins().expect("builtin plugins");

    let session = engine
        .prepare_session(LaunchOptions {
            request: "launch".into(),
            program: Some("/tmp/my-binary".into()),
            adapter: Some("rust".into()),
            extra: serde_json::json!({}),
        })
        .await
        .unwrap();

    assert_eq!(session.adapter_id, "rust");
}

#[tokio::test]
async fn engine_marks_running_and_sets_control_port() {
    let engine = DapEngine::with_builtin_plugins().expect("builtin plugins");
    let session = engine
        .prepare_session(LaunchOptions {
            request: "launch".into(),
            program: Some("main.py".into()),
            adapter: Some("fake".into()),
            extra: serde_json::json!({}),
        })
        .await
        .expect("prepare");

    engine.mark_running(&session).await.expect("running");
    engine
        .set_control_port(&session, 12345)
        .await
        .expect("control port");
}

#[tokio::test]
async fn routes_rust_by_rs_extension() {
    let engine = DapEngine::with_builtin_plugins().expect("builtin plugins");

    let session = engine
        .prepare_session(LaunchOptions {
            request: "launch".into(),
            program: Some("src/main.rs".into()),
            adapter: None,
            extra: serde_json::json!({}),
        })
        .await
        .unwrap();

    assert_eq!(session.adapter_id, "rust");
}
