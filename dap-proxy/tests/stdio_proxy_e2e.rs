use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

fn proxy_bin() -> std::path::PathBuf {
    std::env::var("CARGO_BIN_EXE_dap-proxy")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let exe = std::env::current_exe().expect("current_exe");
            let debug_dir = exe
                .parent()
                .expect("exe parent")
                .parent()
                .expect("debug dir");
            debug_dir.join("dap-proxy")
        })
}

fn adapter_bin() -> std::path::PathBuf {
    std::env::var("CARGO_BIN_EXE_fake-dap-adapter")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let exe = std::env::current_exe().expect("current_exe");
            let debug_dir = exe
                .parent()
                .expect("exe parent")
                .parent()
                .expect("debug dir");
            debug_dir.join("fake-dap-adapter")
        })
}

fn temp_sessions_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dap-proxy-stdio-e2e-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("sessions dir");
    dir
}

#[tokio::test]
async fn proxy_forwards_initialize_to_fake_adapter() {
    let proxy = proxy_bin();
    let adapter = adapter_bin();
    let sessions_dir = temp_sessions_dir();

    let mut child = Command::new(&proxy)
        .arg("--stdio")
        .arg("--adapter-cmd")
        .arg(adapter)
        .env("DAP_SESSIONS_DIR", &sessions_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn dap-proxy");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);

    let request = r#"{"type":"request","seq":1,"command":"initialize","arguments":{}}"#;
    let frame = format!("Content-Length: {}\r\n\r\n{}", request.len(), request);
    stdin.write_all(frame.as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    let response = timeout(Duration::from_secs(5), read_dap_json(&mut reader))
        .await
        .expect("timeout")
        .expect("initialize response");

    assert_eq!(response["type"], "response");
    assert_eq!(response["command"], "initialize");
    assert_eq!(response["success"], true);

    let _ = child.kill().await;
    let _ = std::fs::remove_dir_all(&sessions_dir);
}

#[tokio::test]
async fn proxy_forwards_launch_sequence() {
    let proxy = proxy_bin();
    let adapter = adapter_bin();
    let sessions_dir = temp_sessions_dir();

    let mut child = Command::new(&proxy)
        .arg("--stdio")
        .arg("--adapter-cmd")
        .arg(adapter)
        .env("DAP_SESSIONS_DIR", &sessions_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn dap-proxy");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);

    send_request(&mut stdin, 1, "initialize", r#"{}"#).await;
    let _ = read_until_response(&mut reader, 1).await;

    send_request(&mut stdin, 2, "launch", r#"{"program":"main.py"}"#).await;
    let _ = read_until_response(&mut reader, 2).await;
    let initialized = read_until_event(&mut reader, "initialized").await;
    assert_eq!(initialized["event"], "initialized");

    send_request(&mut stdin, 3, "configurationDone", r#"{}"#).await;
    let _ = read_until_response(&mut reader, 3).await;
    let stopped = read_until_event(&mut reader, "stopped").await;
    assert_eq!(stopped["event"], "stopped");

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

async fn read_until_response(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    request_seq: i64,
) -> serde_json::Value {
    loop {
        let msg = read_dap_json(reader)
            .await
            .expect("message while waiting for response");
        if msg["type"] == "response" && msg["requestSeq"] == request_seq {
            return msg;
        }
    }
}

async fn read_until_event(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    event: &str,
) -> serde_json::Value {
    loop {
        let msg = read_dap_json(reader)
            .await
            .expect("message while waiting for event");
        if msg["type"] == "event" && msg["event"] == event {
            return msg;
        }
    }
}

async fn read_dap_json<R: AsyncBufReadExt + Unpin>(reader: &mut R) -> Option<serde_json::Value> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return None;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().ok();
        }
    }
    let len = content_length?;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await.ok()?;
    serde_json::from_slice(&body).ok()
}
