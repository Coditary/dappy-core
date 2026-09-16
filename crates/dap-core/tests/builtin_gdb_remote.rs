use dap_core::{
    AdapterSpawnOptions, Backend, ControlClient, MultiplexOptions, SessionInitConfig,
    run_session_init,
};
use dap_gdb_remote::testing::MockGdbServer;
use dap_plugin_api::{default_builtin_dir, load_from_file};
use dap_protocol::{Message, Request};
use serde_json::json;
use tokio::time::{Duration, sleep, timeout};

#[tokio::test]
async fn builtin_gdb_remote_initialize_roundtrip_on_duplex() {
    let manifest = load_from_file(&default_builtin_dir().join("gdb-remote/plugin.yaml"))
        .expect("gdb-remote manifest");

    let backend = Backend::spawn_with_options(&manifest.adapter, AdapterSpawnOptions::default())
        .await
        .expect("spawn builtin gdb-remote");

    let (mut read, mut write) = backend.into_duplex().into_channels();
    write
        .send(&Message::Request(Request {
            seq: 1,
            command: "initialize".into(),
            arguments: Some(json!({"adapterID": "gdb-remote"})),
        }))
        .await
        .expect("send initialize");
    write.flush().await.expect("flush");

    let response = timeout(Duration::from_secs(5), read.recv())
        .await
        .expect("initialize timeout")
        .expect("recv")
        .expect("message");

    assert!(
        matches!(&response, Message::Response(r) if r.command.as_deref() == Some("initialize") && r.success),
        "unexpected response: {response:?}"
    );
}

#[tokio::test]
async fn builtin_gdb_remote_session_init_over_control_client() {
    let server = MockGdbServer::start().await;
    let manifest = load_from_file(&default_builtin_dir().join("gdb-remote/plugin.yaml"))
        .expect("gdb-remote manifest");

    let backend = Backend::spawn_with_options(&manifest.adapter, AdapterSpawnOptions::default())
        .await
        .expect("spawn builtin gdb-remote");

    let proxy = dap_core::start_headless_multiplexed_proxy(
        backend.into_duplex(),
        MultiplexOptions {
            control_port: Some(0),
            ..Default::default()
        },
        None,
    )
    .await
    .expect("start headless mux proxy");

    sleep(Duration::from_millis(50)).await;

    let mut client = ControlClient::connect(proxy.control_port.unwrap())
        .await
        .expect("connect control");

    let result = run_session_init(
        &mut client,
        &SessionInitConfig::new("/tmp/dap-rsp-hello")
            .with_adapter_id("gdb-remote")
            .with_manifest(manifest)
            .with_launch_overrides(serde_json::json!({
                "host": "127.0.0.1",
                "port": server.port(),
                "program": "/tmp/dap-rsp-hello",
                "stopAtEntry": true,
            })),
    )
    .await;

    proxy.join.abort();
    let init = result.expect("session init with builtin gdb-remote");
    assert!(init.ready);
}
