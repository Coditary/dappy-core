use std::process::Stdio;
use std::time::Duration;

use dap_core::{ControlClient, LaunchOptions};
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
async fn control_client_threads_and_stack_trace() {
    let proxy = proxy_bin();
    let adapter = adapter_bin();
    let sessions_dir = std::env::temp_dir().join(format!(
        "dap-cli-e2e-{}",
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

    let control_port = read_control_port(child.stderr.take().expect("stderr")).await;
    drive_editor_session(
        child.stdin.take().expect("stdin"),
        child.stdout.take().expect("stdout"),
    )
    .await;
    sleep(Duration::from_millis(100)).await;

    let mut client = ControlClient::connect(control_port)
        .await
        .expect("connect control");
    let threads = client.threads().await.expect("threads");
    assert_eq!(threads["threads"][0]["id"], 1);

    let stack = client.stack_trace(1).await.expect("stackTrace");
    assert_eq!(stack["stackFrames"][0]["name"], "accumulate");

    let scopes = client.scopes(1).await.expect("scopes");
    assert_eq!(scopes["scopes"][0]["name"], "Locals");

    let variables = client.variables(1).await.expect("variables");
    assert_eq!(variables["variables"][0]["name"], "x");

    let eval = client.evaluate("1 + 1", Some(1)).await.expect("evaluate");
    assert_eq!(eval["result"], "eval(1 + 1)");

    let set_var = client
        .set_variable("x", "99", 1)
        .await
        .expect("set variable");
    assert_eq!(set_var["value"], "99");

    let exc = client
        .set_exception_breakpoints(
            &[dap_core::ExceptionBreakpointSpec {
                filter: "uncaught".into(),
                condition: None,
            }],
            true,
        )
        .await
        .expect("set exception breakpoints");
    assert!(exc["breakpoints"].is_array());

    let nav = client
        .navigate(dap_core::NavigationType::StepOver, 1)
        .await
        .expect("step over");
    assert!(nav.success);
    assert_eq!(nav.stop_reason.as_deref(), Some("step"));

    client.disconnect().await.expect("disconnect");
    client.terminate().await.expect("terminate");

    let _ = child.kill().await;
    let _ = std::fs::remove_dir_all(&sessions_dir);
}

#[tokio::test]
async fn control_client_restores_capabilities_on_late_attach() {
    let proxy = proxy_bin();
    let adapter = adapter_bin();
    let sessions_dir = std::env::temp_dir().join(format!(
        "dap-cli-caps-{}",
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

    let control_port = read_control_port(child.stderr.take().expect("stderr")).await;
    drive_editor_session(
        child.stdin.take().expect("stdin"),
        child.stdout.take().expect("stdout"),
    )
    .await;
    sleep(Duration::from_millis(100)).await;

    let client = ControlClient::connect(control_port)
        .await
        .expect("connect control");
    assert!(
        client.capabilities().supports_step_back,
        "late attach should restore initialize capabilities"
    );

    let _ = child.kill().await;
    let _ = std::fs::remove_dir_all(&sessions_dir);
}

#[tokio::test]
async fn routes_python_program_without_explicit_adapter() {
    let mut engine = dap_core::DapEngine::new();
    engine.load_builtin_plugins().expect("load plugins");

    let spawn = engine
        .resolve_adapter_spawn(&LaunchOptions {
            request: "launch".into(),
            program: Some("main.py".into()),
            adapter: None,
            extra: serde_json::json!({}),
        })
        .expect("resolve python");

    assert_eq!(spawn.command, "python3");
    assert!(spawn.args.iter().any(|a| a.contains("debugpy")));
}

async fn read_control_port(stderr: tokio::process::ChildStderr) -> u16 {
    let mut stderr_reader = BufReader::new(stderr);
    let mut line = String::new();
    while timeout(Duration::from_secs(5), stderr_reader.read_line(&mut line))
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
    mut stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
) {
    let mut reader = BufReader::new(stdout);
    send_request(&mut stdin, 1, "initialize", r#"{}"#).await;
    let _ = read_json(&mut reader).await;
    send_request(&mut stdin, 2, "launch", r#"{"program":"main.py"}"#).await;
    let _ = read_json(&mut reader).await;
    let _ = read_json(&mut reader).await;
    send_request(&mut stdin, 3, "configurationDone", r#"{}"#).await;
    let _ = read_json(&mut reader).await;
    let _ = read_json(&mut reader).await;
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
