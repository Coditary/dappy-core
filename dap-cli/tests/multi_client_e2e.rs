mod common;

use std::process::Stdio;
use std::time::Duration;

use common::{assert_ok, lock_tests, path_with_adapter};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::{sleep, timeout};

fn proxy_bin() -> std::path::PathBuf {
    std::env::var("CARGO_BIN_EXE_dap-proxy")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let exe = std::env::current_exe().expect("current_exe");
            exe.parent().expect("parent").parent().expect("debug").join("dap-proxy")
        })
}

fn adapter_bin() -> std::path::PathBuf {
    std::env::var("CARGO_BIN_EXE_fake-dap-adapter")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let exe = std::env::current_exe().expect("current_exe");
            exe.parent().expect("parent").parent().expect("debug").join("fake-dap-adapter")
        })
}

#[tokio::test]
async fn repl_attach_leaves_editor_session_alive() {
    let _guard = lock_tests().await;

    let sessions_dir = std::env::temp_dir().join(format!(
        "dap-multi-client-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");

    let mut proxy = Command::new(proxy_bin())
        .arg("--stdio")
        .arg("--adapter-cmd")
        .arg(adapter_bin())
        .arg("--program")
        .arg("main.py")
        .env("DAP_SESSIONS_DIR", &sessions_dir)
        .env("RUST_LOG", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dap-proxy");

    let stderr = proxy.stderr.take().expect("stderr");
    let mut stderr_reader = BufReader::new(stderr);
    let control_port = read_control_port(&mut stderr_reader).await;

    let mut editor_in = proxy.stdin.take().expect("editor stdin");
    let editor_out = proxy.stdout.take().expect("editor stdout");
    let mut editor_reader = BufReader::new(editor_out);

    drive_editor_launch(&mut editor_in, &mut editor_reader).await;

    let repl_threads = run_repl_command(
        control_port,
        &json!({ "id": 1, "op": "threads" }),
    )
    .await;
    assert_ok(&repl_threads);
    assert!(repl_threads["result"]["threads"].is_array());

    run_repl_command(control_port, &json!({ "id": 2, "op": "quit" })).await;

    sleep(Duration::from_millis(100)).await;

    send_dap_request(&mut editor_in, 20, "threads", r#"{}"#).await;
    let editor_threads = read_dap_message(&mut editor_reader)
        .await
        .expect("editor threads after repl quit");
    assert_eq!(editor_threads["command"], "threads");
    assert_eq!(editor_threads["success"], true);

    let _ = proxy.kill().await;
    let _ = std::fs::remove_dir_all(&sessions_dir);
}

#[tokio::test]
async fn repl_attach_restores_capabilities_from_session() {
    let _guard = lock_tests().await;

    let sessions_dir = std::env::temp_dir().join(format!(
        "dap-multi-client-caps-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");

    let mut proxy = Command::new(proxy_bin())
        .arg("--stdio")
        .arg("--adapter-cmd")
        .arg(adapter_bin())
        .arg("--program")
        .arg("main.py")
        .env("DAP_SESSIONS_DIR", &sessions_dir)
        .env("RUST_LOG", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dap-proxy");

    let stderr = proxy.stderr.take().expect("stderr");
    let mut stderr_reader = BufReader::new(stderr);
    let control_port = read_control_port(&mut stderr_reader).await;

    let mut editor_in = proxy.stdin.take().expect("editor stdin");
    let editor_out = proxy.stdout.take().expect("editor stdout");
    let mut editor_reader = BufReader::new(editor_out);
    drive_editor_launch(&mut editor_in, &mut editor_reader).await;

    let caps = run_repl_command(
        control_port,
        &json!({ "id": 1, "op": "capabilities" }),
    )
    .await;
    assert_ok(&caps);
    assert_eq!(caps["result"]["supportsStepBack"], true);

    let step_back = run_repl_command(
        control_port,
        &json!({ "id": 2, "op": "step_back" }),
    )
    .await;
    assert_ok(&step_back);

    run_repl_command(control_port, &json!({ "id": 3, "op": "quit" })).await;

    let _ = proxy.kill().await;
    let _ = std::fs::remove_dir_all(&sessions_dir);
}

#[tokio::test]
async fn repl_attach_imports_editor_breakpoints() {
    let _guard = lock_tests().await;

    let sessions_dir = std::env::temp_dir().join(format!(
        "dap-multi-client-bp-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&sessions_dir).expect("sessions dir");

    let mut proxy = Command::new(proxy_bin())
        .arg("--stdio")
        .arg("--adapter-cmd")
        .arg(adapter_bin())
        .arg("--program")
        .arg("main.py")
        .env("DAP_SESSIONS_DIR", &sessions_dir)
        .env("RUST_LOG", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dap-proxy");

    let stderr = proxy.stderr.take().expect("stderr");
    let mut stderr_reader = BufReader::new(stderr);
    let control_port = read_control_port(&mut stderr_reader).await;

    let mut editor_in = proxy.stdin.take().expect("editor stdin");
    let editor_out = proxy.stdout.take().expect("editor stdout");
    let mut editor_reader = BufReader::new(editor_out);
    drive_editor_launch(&mut editor_in, &mut editor_reader).await;

    send_dap_request(
        &mut editor_in,
        10,
        "setBreakpoints",
        r#"{"source":{"path":"/fake/main.py"},"breakpoints":[{"line":42}]}"#,
    )
    .await;
    let _ = read_dap_message(&mut editor_reader).await;

    let breakpoints = run_repl_command(
        control_port,
        &json!({ "id": 1, "op": "breakpoints" }),
    )
    .await;
    assert_ok(&breakpoints);
    let entries = breakpoints["result"]["/fake/main.py"]
        .as_array()
        .expect("breakpoint entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["line"], 42);

    let synced = run_repl_command(control_port, &json!({ "id": 2, "op": "sync" })).await;
    assert_ok(&synced);
    let synced_entries = synced["result"]["breakpoints"]["merged"]["/fake/main.py"]
        .as_array()
        .expect("synced breakpoint entries");
    assert_eq!(synced_entries.len(), 1);
    assert_eq!(synced_entries[0]["line"], 42);

    run_repl_command(control_port, &json!({ "id": 3, "op": "quit" })).await;

    let _ = proxy.kill().await;
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

async fn drive_editor_launch(
    editor_in: &mut tokio::process::ChildStdin,
    editor_out: &mut BufReader<tokio::process::ChildStdout>,
) {
    send_dap_request(editor_in, 1, "initialize", r#"{}"#).await;
    let _ = read_dap_message(editor_out).await;
    send_dap_request(editor_in, 2, "launch", r#"{"program":"main.py"}"#).await;
    let _ = read_dap_message(editor_out).await;
    let _ = read_dap_message(editor_out).await;
    send_dap_request(editor_in, 3, "configurationDone", r#"{}"#).await;
    let _ = read_dap_message(editor_out).await;
    let _ = read_dap_message(editor_out).await;
}

async fn run_repl_command(control_port: u16, request: &serde_json::Value) -> serde_json::Value {
    let mut child = Command::new(common::cli_bin())
        .args([
            "debug",
            "repl",
            "--ndjson",
            "--control-port",
            &control_port.to_string(),
        ])
        .env("PATH", path_with_adapter())
        .env("RUST_LOG", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn repl attach");

    let mut stdin = child.stdin.take().expect("repl stdin");
    let stdout = child.stdout.take().expect("repl stdout");
    let mut stdout_reader = BufReader::new(stdout);

    let _ready = common::read_json_line(&mut stdout_reader)
        .await
        .expect("ready line");

    let line = serde_json::to_string(request).expect("serialize request");
    stdin.write_all(line.as_bytes()).await.expect("write request");
    stdin.write_all(b"\n").await.expect("write newline");
    stdin.flush().await.expect("flush request");

    let response = common::read_json_line(&mut stdout_reader)
        .await
        .expect("response line");

    if request.get("op").and_then(|v| v.as_str()) != Some("quit") {
        stdin
            .write_all(br#"{"id":99,"op":"quit"}"#)
            .await
            .expect("write quit");
        stdin.write_all(b"\n").await.expect("write newline");
        stdin.flush().await.expect("flush quit");
    }

    let _ = child.wait().await;
    response
}

async fn send_dap_request(
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

async fn read_dap_message<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
) -> Option<serde_json::Value> {
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
