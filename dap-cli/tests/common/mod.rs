use std::path::PathBuf;
use std::process::Stdio;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

static TEST_LOCK: Mutex<()> = Mutex::const_new(());

pub async fn lock_tests() -> tokio::sync::MutexGuard<'static, ()> {
    TEST_LOCK.lock().await
}

pub fn cli_bin() -> PathBuf {
    std::env::var("CARGO_BIN_EXE_dap-cli")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let exe = std::env::current_exe().expect("current_exe");
            let debug_dir = exe.parent().expect("parent").parent().expect("debug");
            debug_dir.join("dap-cli")
        })
}

pub fn adapter_bin_dir() -> PathBuf {
    std::env::var("CARGO_BIN_EXE_fake-dap-adapter")
        .ok()
        .map(PathBuf::from)
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_else(|| {
            let exe = std::env::current_exe().expect("current_exe");
            exe.parent().expect("parent").parent().expect("debug").to_path_buf()
        })
}

pub fn path_with_adapter() -> String {
    let adapter_dir = adapter_bin_dir();
    let current = std::env::var("PATH").unwrap_or_default();
    format!("{}:{}", adapter_dir.display(), current)
}

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/fixtures")
        .join(name)
}

#[allow(dead_code)]
pub struct ReplNdjsonSession {
    child: Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
}

#[allow(dead_code)]
impl ReplNdjsonSession {
    pub async fn spawn() -> Self {
        let program = fixture_path("main.py");
        Self::spawn_with_program_in_dir(
            program.to_str().expect("fixture path"),
            program.parent().expect("fixture parent"),
        )
        .await
    }

    pub async fn spawn_with_program(program: &str) -> Self {
        let mut child = Command::new(cli_bin())
            .args(["debug", "repl", "--ndjson", "--adapter", "fake", program])
            .env("PATH", path_with_adapter())
            .env("RUST_LOG", "off")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn dap-cli repl");

        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        }
    }

    pub async fn spawn_with_program_in_dir(program: &str, cwd: &std::path::Path) -> Self {
        let mut child = Command::new(cli_bin())
            .args(["debug", "repl", "--ndjson", "--adapter", "fake", program])
            .env("PATH", path_with_adapter())
            .env("RUST_LOG", "off")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn dap-cli repl");

        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        }
    }

    pub async fn read_response(&mut self) -> Value {
        read_json_line(&mut self.stdout)
            .await
            .expect("read repl response")
    }

    pub async fn send(&mut self, request: &Value) -> Value {
        let line = serde_json::to_string(request).expect("serialize request");
        self.stdin
            .write_all(line.as_bytes())
            .await
            .expect("write request");
        self.stdin.write_all(b"\n").await.expect("write newline");
        self.stdin.flush().await.expect("flush request");
        self.read_response().await
    }

    pub async fn finish(mut self) {
        let _ = self.child.kill().await;
    }
}

pub async fn read_json_line<R: AsyncBufReadExt + Unpin>(reader: &mut R) -> Option<Value> {
    let mut line = String::new();
    while reader.read_line(&mut line).await.expect("read line") > 0 {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            line.clear();
            continue;
        }
        return serde_json::from_str(trimmed).ok();
    }
    None
}

pub fn assert_ok(response: &Value) {
    assert_eq!(
        response.get("ok").and_then(Value::as_bool),
        Some(true),
        "expected ok response, got: {}",
        response
    );
}
