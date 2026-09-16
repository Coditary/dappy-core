use std::process::Stdio;
use std::time::Duration;

use dap_core::ControlClient;
use instance_manager::SessionStore;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::{sleep, timeout};

fn proxy_bin() -> std::path::PathBuf {
    std::env::var("CARGO_BIN_EXE_dap-proxy")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let exe = std::env::current_exe().expect("current_exe");
            let debug_dir = exe.parent().expect("parent").parent().expect("debug");
            debug_dir.join("dap-proxy")
        })
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

#[tokio::test]
async fn start_debugging_spawns_child_session() {
    let proxy = proxy_bin();
    let adapter = adapter_bin();
    let scope = format!(
        "child-session-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let sessions_dir = std::env::temp_dir().join(format!("dap-child-e2e-{}", scope));
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");

    let mut child = Command::new(&proxy)
        .arg("--stdio")
        .arg("--adapter-cmd")
        .arg(&adapter)
        .arg("--fake-emit-start-debugging")
        .arg("--child-sessions")
        .arg("--control-port")
        .arg("0")
        .arg("--scope")
        .arg(&scope)
        .env("DAP_SESSIONS_DIR", &sessions_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dap-proxy");

    let stderr = child.stderr.take().expect("stderr");
    let mut stderr_reader = BufReader::new(stderr);
    let mut parent_port = None;
    let mut parent_instance_id = None;
    let mut line = String::new();
    while timeout(Duration::from_secs(30), stderr_reader.read_line(&mut line))
        .await
        .expect("stderr timeout")
        .expect("stderr line")
        > 0
    {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line.trim()) {
            if let (Some(port), Some(instance_id)) = (
                json.get("controlPort").and_then(|v| v.as_u64()),
                json.get("instanceId").and_then(|v| v.as_str()),
            ) {
                parent_port = Some(port as u16);
                parent_instance_id = Some(instance_id.to_string());
                break;
            }
        }
        line.clear();
    }
    let parent_port = parent_port.expect("parent controlPort");
    let parent_instance_id = parent_instance_id.expect("parent instanceId");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = BufReader::new(stdout);

    send_request(&mut stdin, 1, "initialize", r#"{}"#).await;
    let _ = read_json(&mut stdout_reader).await;
    send_request(&mut stdin, 2, "launch", r#"{"program":"main.py"}"#).await;
    let _ = read_json(&mut stdout_reader).await;
    let _ = read_json(&mut stdout_reader).await;
    send_request(&mut stdin, 3, "configurationDone", r#"{}"#).await;
    let _ = read_json(&mut stdout_reader).await;
    let _ = read_json(&mut stdout_reader).await;

    let store = SessionStore::open(&sessions_dir).expect("open store");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut child_record = None;
    while std::time::Instant::now() < deadline {
        let sessions = store.list_active().expect("list sessions");
        child_record = sessions
            .into_iter()
            .find(|s| s.scope.as_deref() == Some(scope.as_str()) && s.control_port != parent_port);
        if child_record.is_some() {
            break;
        }
        sleep(Duration::from_millis(200)).await;
    }
    let child_record = child_record.expect("child session in store");
    assert_eq!(
        child_record.parent_id.as_deref(),
        Some(parent_instance_id.as_str())
    );

    let mut client = ControlClient::connect(child_record.control_port)
        .await
        .expect("connect child");
    let threads = timeout(
        Duration::from_secs(5),
        client.dap_request("threads", Some(serde_json::json!({}))),
    )
    .await
    .expect("threads timeout")
    .expect("threads request");
    match threads {
        dap_protocol::Message::Response(resp) => assert!(resp.success),
        other => panic!("expected threads response, got {:?}", other),
    }

    let _ = child.kill().await;
    let _ = std::fs::remove_dir_all(&sessions_dir);
}

async fn send_request(
    stdin: &mut tokio::process::ChildStdin,
    seq: i64,
    command: &str,
    arguments: &str,
) {
    let request = format!(
        r#"{{"type":"request","seq":{},"command":"{}","arguments":{}}}"#,
        seq, command, arguments
    );
    let frame = format!("Content-Length: {}\r\n\r\n{}", request.len(), request);
    stdin.write_all(frame.as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();
}

async fn read_json<R: AsyncBufReadExt + Unpin>(reader: &mut R) -> Option<serde_json::Value> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return None;
        }
        if line.trim().is_empty() {
            break;
        }
        if let Some(rest) = line.trim().strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().ok();
        }
    }
    let len = content_length?;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await.ok()?;
    serde_json::from_slice(&body).ok()
}
