use std::process::Stdio;
use std::time::Duration;

use dap_core::ControlClient;
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
async fn control_attach_threads_while_editor_connected() {
    let proxy = proxy_bin();
    let adapter = adapter_bin();
    let sessions_dir = std::env::temp_dir().join(format!(
        "dap-proxy-e2e-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");

    let mut child = Command::new(&proxy)
        .arg("--stdio")
        .arg("--adapter-cmd")
        .arg(adapter)
        .arg("--control-port")
        .arg("0")
        .env("DAP_SESSIONS_DIR", &sessions_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dap-proxy");

    let stderr = child.stderr.take().expect("stderr");
    let mut stderr_reader = BufReader::new(stderr);
    let control_port = read_control_port(&mut stderr_reader).await;

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = BufReader::new(stdout);

    drive_editor_session(&mut stdin, &mut stdout_reader).await;
    sleep(Duration::from_millis(100)).await;

    let mut client = ControlClient::connect(control_port)
        .await
        .expect("connect control");
    let threads = client.threads().await.expect("threads");
    assert_eq!(threads["threads"][0]["id"], 1);

    let _ = child.kill().await;
    let _ = std::fs::remove_dir_all(&sessions_dir);
}

async fn read_control_port(stderr: &mut BufReader<tokio::process::ChildStderr>) -> u16 {
    let mut line = String::new();
    while timeout(Duration::from_secs(5), stderr.read_line(&mut line))
        .await
        .expect("stderr timeout")
        .expect("stderr line")
        > 0
    {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line.trim()) {
            if let Some(port) = json.get("controlPort").and_then(|v| v.as_u64()) {
                return port as u16;
            }
        }
        line.clear();
    }
    panic!("controlPort json on stderr");
}

async fn drive_editor_session(
    stdin: &mut tokio::process::ChildStdin,
    stdout: &mut BufReader<tokio::process::ChildStdout>,
) {
    send_request(stdin, 1, "initialize", r#"{}"#).await;
    let _ = read_json(stdout).await;
    send_request(stdin, 2, "launch", r#"{"program":"main.py"}"#).await;
    let _ = read_json(stdout).await;
    let _ = read_json(stdout).await;
    send_request(stdin, 3, "configurationDone", r#"{}"#).await;
    let _ = read_json(stdout).await;
    let _ = read_json(stdout).await;
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
