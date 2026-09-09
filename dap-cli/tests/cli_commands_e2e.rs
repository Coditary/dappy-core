mod common;

use std::process::Stdio;
use std::time::Duration;

use common::{lock_tests, path_with_adapter};
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

struct ProxySession {
    child: tokio::process::Child,
    control_port: u16,
    sessions_dir: std::path::PathBuf,
    instance_id: Option<String>,
}

impl ProxySession {
    async fn spawn() -> Self {
        let sessions_dir = std::env::temp_dir().join(format!(
            "dap-cli-e2e-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&sessions_dir).expect("sessions dir");

        let mut child = Command::new(proxy_bin())
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

        let stderr = child.stderr.take().expect("stderr");
        let control_port = read_control_port(BufReader::new(stderr)).await;

        let mut editor_in = child.stdin.take().expect("stdin");
        let editor_out = child.stdout.take().expect("stdout");
        drive_editor_launch(&mut editor_in, BufReader::new(editor_out)).await;

        sleep(Duration::from_millis(100)).await;

        Self {
            child,
            control_port,
            sessions_dir,
            instance_id: None,
        }
    }

    async fn run_cli(&self, args: &[&str]) -> std::process::Output {
        Command::new(common::cli_bin())
            .args(args)
            .arg("--control-port")
            .arg(self.control_port.to_string())
            .env("DAP_SESSIONS_DIR", &self.sessions_dir)
            .env("PATH", path_with_adapter())
            .env("RUST_LOG", "off")
            .output()
            .await
            .expect("run cli")
    }

    async fn finish(mut self) {
        let _ = self.child.kill().await;
        let _ = std::fs::remove_dir_all(&self.sessions_dir);
    }
}

#[tokio::test]
async fn cli_debug_commands_cover_control_plane() {
    let _guard = lock_tests().await;
    let session = ProxySession::spawn().await;

    let threads = session.run_cli(&["debug", "threads"]).await;
    assert!(threads.status.success());
    assert!(threads.stdout.windows(4).any(|w| w == b"main"));

    let stack = session.run_cli(&["debug", "stack-trace", "1"]).await;
    assert!(stack.status.success());
    assert!(stack.stdout.windows(4).any(|w| w == b"main"));

    let status = session.run_cli(&["debug", "status"]).await;
    assert!(status.status.success());

    let eval = session
        .run_cli(&["debug", "evaluate", "1 + 1", "--frame-id", "1"])
        .await;
    assert!(eval.status.success());

    let scopes = session.run_cli(&["debug", "scopes", "1"]).await;
    assert!(scopes.status.success());
    assert!(scopes.stdout.windows(6).any(|w| w == b"Locals"));

    let vars = session.run_cli(&["debug", "variables", "1"]).await;
    assert!(vars.status.success());

    let bps = session
        .run_cli(&["debug", "set-breakpoints", "/fake/main.py", "-b", "1"])
        .await;
    assert!(bps.status.success());

    let list = session.run_cli(&["session", "list"]).await;
    assert!(list.status.success());

    let sessions = session.run_cli(&["debug", "sessions"]).await;
    assert!(sessions.status.success());

    let instance_id = serde_json::from_slice::<Vec<instance_manager::SessionRecord>>(&list.stdout)
        .expect("parse sessions")
        .into_iter()
        .next()
        .map(|record| record.instance_id)
        .expect("session id");

    let show = session
        .run_cli(&["session", "show", &instance_id])
        .await;
    assert!(show.status.success());

    let plugins = session.run_cli(&["plugin", "list"]).await;
    assert!(plugins.status.success());
    assert!(plugins.stdout.windows(4).any(|w| w == b"fake"));

    let attach = session
        .run_cli(&["debug", "attach", "--port", &session.control_port.to_string()])
        .await;
    assert!(attach.status.success());

    let kill = session
        .run_cli(&["session", "kill", &instance_id])
        .await;
    assert!(kill.status.success());

    session.finish().await;
}

#[tokio::test]
async fn cli_json_mode_emits_json() {
    let _guard = lock_tests().await;
    let session = ProxySession::spawn().await;

    let output = session.run_cli(&["--json", "debug", "threads"]).await;
    assert!(output.status.success());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse json output");

    assert!(value.is_array() || value.get("threads").is_some());

    let _ = session.run_cli(&["debug", "stop"]).await;
    session.finish().await;
}

async fn read_control_port(mut stderr: BufReader<tokio::process::ChildStderr>) -> u16 {
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
    mut editor_out: BufReader<tokio::process::ChildStdout>,
) {
    send_dap_request(editor_in, 1, "initialize", r#"{}"#).await;
    let _ = read_dap_message(&mut editor_out).await;
    send_dap_request(editor_in, 2, "launch", r#"{"program":"main.py"}"#).await;
    let _ = read_dap_message(&mut editor_out).await;
    let _ = read_dap_message(&mut editor_out).await;
    send_dap_request(editor_in, 3, "configurationDone", r#"{}"#).await;
    let _ = read_dap_message(&mut editor_out).await;
    let _ = read_dap_message(&mut editor_out).await;
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

#[tokio::test]
async fn cli_debug_start_prepares_session() {
    let _guard = lock_tests().await;
    let output = tokio::process::Command::new(common::cli_bin())
        .args([
            "debug",
            "start",
            "--program",
            "main.py",
            "--adapter",
            "fake",
        ])
        .env("PATH", path_with_adapter())
        .output()
        .await
        .expect("debug start");
    assert!(output.status.success());
    assert!(output.stdout.windows(4).any(|w| w == b"fake"));
}
