//! High-level DAP client helpers for custom frontends (TUI, scripts, integration tests).
//!
//! [`DapClient`] is the same type as [`ControlClient`](crate::ControlClient): a buffered
//! request/response client over any DAP duplex (proxy stdio, control TCP, or adapter pipes).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use dap_protocol::DuplexChannel;
use tokio::process::{Child, Command};

use crate::control_client::ControlClient;
use crate::session_init::{SessionInitConfig, build_initialize_arguments, build_launch_arguments};

/// Buffered DAP client for frontends that drive a debug session over stdio or TCP.
pub type DapClient = ControlClient;

/// Options for spawning `dap-proxy --stdio` as a child process.
#[derive(Debug, Clone)]
pub struct ProxyStdioOptions {
    /// Program path passed to `--program` (adapter routing).
    pub program: Option<String>,
    /// Builtin adapter id (`--adapter`).
    pub adapter: Option<String>,
    /// Override adapter command (`--adapter-cmd`).
    pub adapter_cmd: Option<Vec<String>>,
    /// Disable the control attach listener (`--no-control`). Typical for sole-client TUIs.
    pub no_control: bool,
    /// Plugin manifest directory (`--plugins-dir`).
    pub plugins_dir: Option<PathBuf>,
    /// `dap-proxy` executable. Defaults to `CARGO_BIN_EXE_dap-proxy` in tests, else `dap-proxy` on `PATH`.
    pub proxy_bin: Option<PathBuf>,
    /// Extra arguments appended after built-in flags.
    pub extra_args: Vec<String>,
}

impl Default for ProxyStdioOptions {
    fn default() -> Self {
        Self {
            program: None,
            adapter: None,
            adapter_cmd: None,
            no_control: true,
            plugins_dir: None,
            proxy_bin: None,
            extra_args: Vec::new(),
        }
    }
}

impl ProxyStdioOptions {
    pub fn program(program: impl Into<String>) -> Self {
        Self {
            program: Some(program.into()),
            ..Self::default()
        }
    }

    pub fn with_adapter(mut self, adapter: impl Into<String>) -> Self {
        self.adapter = Some(adapter.into());
        self
    }

    pub fn with_adapter_cmd(mut self, cmd: Vec<String>) -> Self {
        self.adapter_cmd = Some(cmd);
        self
    }

    pub fn with_control_port(mut self) -> Self {
        self.no_control = false;
        self
    }
}

/// A running `dap-proxy` child and a DAP client connected to its stdio.
pub struct ProxyStdioSession {
    pub child: Child,
    pub client: DapClient,
}

/// Spawn `dap-proxy --stdio` and return a [`DapClient`] wired to its stdin/stdout.
pub async fn spawn_proxy_stdio(opts: ProxyStdioOptions) -> Result<ProxyStdioSession> {
    let proxy_bin = opts
        .proxy_bin
        .or_else(default_proxy_bin)
        .context("dap-proxy executable not found; set proxy_bin or install dap-proxy on PATH")?;

    let mut command = Command::new(&proxy_bin);
    command.arg("--stdio");
    if opts.no_control {
        command.arg("--no-control");
    }
    if let Some(program) = &opts.program {
        command.arg("--program").arg(program);
    }
    if let Some(adapter) = &opts.adapter {
        command.arg("--adapter").arg(adapter);
    }
    if let Some(adapter_cmd) = &opts.adapter_cmd {
        command.arg("--adapter-cmd");
        command.args(adapter_cmd);
    }
    if let Some(dir) = &opts.plugins_dir {
        command.arg("--plugins-dir").arg(dir);
    }
    command.args(&opts.extra_args);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::inherit());

    let mut child = command
        .spawn()
        .with_context(|| format!("spawn dap-proxy at {}", proxy_bin.display()))?;

    let stdin = child.stdin.take().context("dap-proxy stdin")?;
    let stdout = child.stdout.take().context("dap-proxy stdout")?;
    let duplex = DuplexChannel::from_streams(stdout, stdin);
    let client = DapClient::from_duplex(duplex);

    Ok(ProxyStdioSession { child, client })
}

/// Build DAP `initialize` arguments for a frontend session.
pub fn initialize_arguments(config: &SessionInitConfig) -> serde_json::Value {
    build_initialize_arguments(config)
}

/// Build DAP `launch` arguments for a frontend session (adapter-aware).
pub fn launch_arguments(config: &SessionInitConfig) -> serde_json::Value {
    build_launch_arguments(config)
}

fn default_proxy_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_dap-proxy") {
        return Some(PathBuf::from(path));
    }
    which_on_path("dap-proxy")
}

fn which_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = Path::new(&dir).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
