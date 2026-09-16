use dap_core::{
    AdapterGuard, Backend, MultiplexOptions, ProxyPluginContext, SUPPORTS_DAP_PROXY_PLUGIN_INFO_REQUEST,
    connect_control_client, roundtrip_request,
};
use dap_plugin_api::{AdapterSpawn, SpawnTransport, load_from_file};
use dap_protocol::{DuplexChannel, Message};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::time::{Duration, sleep};

#[tokio::test]
async fn control_client_gets_own_response_while_editor_session_live() {
    let adapter = adapter_bin();
    let backend = Backend::spawn(&AdapterSpawn {
        transport: SpawnTransport::Stdio,
        command: adapter.to_string_lossy().into_owned(),
        args: vec![],
    })
    .await
    .expect("spawn fake adapter");

    let (editor_read, mut driver_write) = tokio::io::duplex(4096);
    let (mut driver_read, editor_write) = tokio::io::duplex(4096);
    let editor = DuplexChannel::from_streams(editor_read, editor_write);
    let (editor_done_tx, editor_done_rx) = tokio::sync::oneshot::channel();

    let driver = tokio::spawn(async move {
        send_json(
            &mut driver_write,
            r#"{"type":"request","seq":1,"command":"initialize","arguments":{}}"#,
        )
        .await;
        let _ = read_json(&mut driver_read).await;
        send_json(
            &mut driver_write,
            r#"{"type":"request","seq":2,"command":"launch","arguments":{"program":"main.py"}}"#,
        )
        .await;
        let _ = read_json(&mut driver_read).await;
        let _ = read_json(&mut driver_read).await;
        let _ = editor_done_rx.await;
    });

    let proxy = dap_core::start_multiplexed_proxy(
        editor,
        backend.into_duplex(),
        MultiplexOptions {
            control_port: Some(0),
            ..Default::default()
        },
        None,
    )
    .await
    .expect("start mux proxy");

    sleep(Duration::from_millis(50)).await;

    let control = connect_control_client(proxy.control_port.unwrap())
        .await
        .expect("connect control");
    let (mut control_read, mut control_write) = control.into_channels();
    let response = roundtrip_request(&mut control_read, &mut control_write, 10, "threads", None)
        .await
        .expect("threads response");
    assert!(matches!(
        response,
        Message::Response(resp) if resp.command.as_deref() == Some("threads")
    ));

    let _ = editor_done_tx.send(());
    driver.await.expect("editor driver");
    proxy.join.abort();
}

#[tokio::test]
async fn editor_disconnect_completes_mux_proxy() {
    let adapter = adapter_bin();
    let backend = Backend::spawn(&AdapterSpawn {
        transport: SpawnTransport::Stdio,
        command: adapter.to_string_lossy().into_owned(),
        args: vec![],
    })
    .await
    .expect("spawn fake adapter");

    let (client_write, server_read) = tokio::io::duplex(4096);
    let (server_write, client_read) = tokio::io::duplex(4096);
    let editor = DuplexChannel::from_streams(server_read, server_write);
    let (backend_duplex, mut adapter_guard) = AdapterGuard::from_backend(backend);

    let proxy = dap_core::start_multiplexed_proxy(
        editor,
        backend_duplex,
        MultiplexOptions {
            control_port: Some(0),
            ..Default::default()
        },
        None,
    )
    .await
    .expect("start mux proxy");

    drop(client_write);
    drop(client_read);

    tokio::time::timeout(Duration::from_secs(5), proxy.join)
        .await
        .expect("proxy should stop after editor disconnect")
        .expect("proxy join task")
        .expect("proxy shutdown");

    adapter_guard.shutdown().await;
}

#[tokio::test]
async fn editor_idle_timeout_completes_mux_proxy() {
    let adapter = adapter_bin();
    let backend = Backend::spawn(&AdapterSpawn {
        transport: SpawnTransport::Stdio,
        command: adapter.to_string_lossy().into_owned(),
        args: vec![],
    })
    .await
    .expect("spawn fake adapter");

    let (_client_write, server_read) = tokio::io::duplex(4096);
    let (server_write, _client_read) = tokio::io::duplex(4096);
    let editor = DuplexChannel::from_streams(server_read, server_write);
    let (backend_duplex, mut adapter_guard) = AdapterGuard::from_backend(backend);

    let proxy = dap_core::start_multiplexed_proxy(
        editor,
        backend_duplex,
        MultiplexOptions {
            control_port: Some(0),
            client_idle_timeout: Some(Duration::from_millis(200)),
            ..Default::default()
        },
        None,
    )
    .await
    .expect("start mux proxy");

    tokio::time::timeout(Duration::from_secs(5), proxy.join)
        .await
        .expect("proxy should stop after editor idle timeout")
        .expect("proxy join task")
        .expect("proxy shutdown");

    adapter_guard.shutdown().await;
}

#[tokio::test]
async fn initialize_response_advertises_proxy_plugin_info() {
    let adapter = adapter_bin();
    let backend = Backend::spawn(&AdapterSpawn {
        transport: SpawnTransport::Stdio,
        command: adapter.to_string_lossy().into_owned(),
        args: vec![],
    })
    .await
    .expect("spawn fake adapter");

    let manifest = load_from_file(
        dap_plugin_api::default_builtin_dir()
            .join("python")
            .join("plugin.yaml")
            .as_path(),
    )
    .expect("python plugin manifest");

    let (editor_read, mut driver_write) = tokio::io::duplex(4096);
    let (mut driver_read, editor_write) = tokio::io::duplex(4096);
    let editor = DuplexChannel::from_streams(editor_read, editor_write);
    let (editor_done_tx, editor_done_rx) = tokio::sync::oneshot::channel();

    let driver = tokio::spawn(async move {
        send_json(
            &mut driver_write,
            r#"{"type":"request","seq":1,"command":"initialize","arguments":{}}"#,
        )
        .await;
        let init = read_json(&mut driver_read).await;
        assert_eq!(
            init["body"][SUPPORTS_DAP_PROXY_PLUGIN_INFO_REQUEST],
            serde_json::Value::Bool(true)
        );
        send_json(
            &mut driver_write,
            r#"{"type":"request","seq":2,"command":"launch","arguments":{"program":"main.py"}}"#,
        )
        .await;
        let _ = read_json(&mut driver_read).await;
        let _ = read_json(&mut driver_read).await;
        let _ = editor_done_rx.await;
    });

    let proxy = dap_core::start_multiplexed_proxy(
        editor,
        backend.into_duplex(),
        MultiplexOptions {
            control_port: Some(0),
            plugin_context: Some(ProxyPluginContext::new(manifest)),
            ..Default::default()
        },
        None,
    )
    .await
    .expect("start mux proxy");

    sleep(Duration::from_millis(50)).await;

    let control = connect_control_client(proxy.control_port.unwrap())
        .await
        .expect("connect control");
    let (mut control_read, mut control_write) = control.into_channels();
    let response =
        roundtrip_request(&mut control_read, &mut control_write, 10, "dapProxyPluginInfo", None)
            .await
            .expect("plugin info response");
    let Message::Response(resp) = response else {
        panic!("expected response");
    };
    let body = resp.body.expect("body");
    assert_eq!(body["pluginId"], "python");
    assert!(body.get("attachment").is_some());

    let _ = editor_done_tx.send(());
    driver.await.expect("editor driver");
    proxy.join.abort();
}

fn adapter_bin() -> std::path::PathBuf {
    std::env::var("CARGO_BIN_EXE_fake-dap-adapter")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let exe = std::env::current_exe().expect("current_exe");
            let debug_dir = exe.parent().expect("parent").parent().expect("debug");
            debug_dir.join("fake-dap-adapter")
        })
}

async fn send_json(writer: &mut tokio::io::DuplexStream, body: &str) {
    let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    writer.write_all(frame.as_bytes()).await.unwrap();
    writer.flush().await.unwrap();
}

async fn read_json(reader: &mut tokio::io::DuplexStream) -> serde_json::Value {
    let mut buf_reader = BufReader::new(reader);
    let mut content_length = None;
    let mut header = String::new();
    while {
        header.clear();
        buf_reader.read_line(&mut header).await.unwrap();
        let trimmed = header.trim();
        if trimmed.is_empty() {
            false
        } else {
            if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
                content_length = rest.trim().parse().ok();
            }
            true
        }
    } {}
    let len = content_length.expect("content length");
    let mut body = vec![0u8; len];
    buf_reader.read_exact(&mut body).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}
