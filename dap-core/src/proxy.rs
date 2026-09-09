use std::path::{Path, PathBuf};
use std::process::Stdio;

use std::sync::Arc;

use anyhow::{Context, Result};
use dap_plugin_api::{AdapterSpawn, SpawnTransport};
use dap_protocol::{DuplexChannel, ProtocolError, ReadChannel, WriteChannel};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::debug;

/// Options for spawning a stdio debug adapter subprocess.
#[derive(Debug, Clone, Copy, Default)]
pub struct AdapterSpawnOptions {
    /// Start the adapter in a new session (`setsid`) so it and its debuggee
    /// cannot steal the terminal foreground process group from the CLI REPL.
    pub new_session: bool,
}

/// Spawned debug adapter backend with stdio pipes.
pub struct Backend {
    duplex: DuplexChannel,
    adapter_process: Option<tokio::process::Child>,
}

/// Best-effort kill for a spawned stdio adapter subprocess.
pub fn kill_adapter_process(adapter_process: &mut Option<tokio::process::Child>) {
    if let Some(process) = adapter_process {
        let _ = process.start_kill();
    }
}

impl Backend {
    pub async fn spawn(adapter: &AdapterSpawn) -> Result<Self> {
        Self::spawn_with_options(adapter, AdapterSpawnOptions::default()).await
    }

    pub async fn spawn_with_options(
        adapter: &AdapterSpawn,
        options: AdapterSpawnOptions,
    ) -> Result<Self> {
        match adapter.transport {
            SpawnTransport::Stdio => {
                Self::spawn_stdio(&adapter.command, &adapter.args, options.new_session).await
            }
            SpawnTransport::Tcp => {
                let (host, port) = adapter
                    .command
                    .rsplit_once(':')
                    .and_then(|(host, port)| port.parse().ok().map(|port| (host, port)))
                    .context("invalid tcp adapter command host:port")?;
                Self::spawn_tcp(host, port).await
            }
        }
    }

    pub async fn spawn_tcp(host: &str, port: u16) -> Result<Self> {
        debug!(host, port, "connecting tcp debug adapter");
        let stream = tokio::net::TcpStream::connect((host, port))
            .await
            .with_context(|| format!("failed to connect tcp adapter at {host}:{port}"))?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            duplex: DuplexChannel::from_streams(reader, writer),
            adapter_process: None,
        })
    }

    pub async fn spawn_stdio(command: &str, args: &[String], new_session: bool) -> Result<Self> {
        debug!(command, ?args, new_session, "spawning debug adapter");

        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Keep the debuggee out of the shell's foreground process group so an
        // interactive REPL can read /dev/tty without `zsh: suspended (tty input)`.
        #[cfg(unix)]
        if new_session {
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }

        let mut process = cmd
            .spawn()
            .with_context(|| format!("failed to spawn adapter: {command}"))?;

        let stdin = process
            .stdin
            .take()
            .context("adapter process missing stdin")?;
        let stdout = process
            .stdout
            .take()
            .context("adapter process missing stdout")?;
        if let Some(stderr) = process.stderr.take() {
            tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!(target: "adapter_stderr", "{line}");
                }
            });
        }

        Ok(Self {
            duplex: DuplexChannel::from_streams(stdout, stdin),
            adapter_process: Some(process),
        })
    }

    pub fn detach_adapter(self) -> (DuplexChannel, Option<tokio::process::Child>) {
        (self.duplex, self.adapter_process)
    }

    pub fn kill_adapter(&mut self) {
        kill_adapter_process(&mut self.adapter_process);
    }

    pub fn duplex(&self) -> &DuplexChannel {
        &self.duplex
    }

    /// Returns only the DAP channel. Prefer [`Self::detach_adapter`] so the adapter
    /// child can be killed explicitly on shutdown.
    pub fn into_duplex(self) -> DuplexChannel {
        let _ = self.adapter_process;
        self.duplex
    }

    pub fn into_channels(self) -> (ReadChannel, WriteChannel) {
        self.duplex.into_channels()
    }
}

/// Transparent bidirectional forward between client and backend (1:1, no mux).
pub async fn run_transparent_proxy(client: DuplexChannel, backend: DuplexChannel) -> Result<()> {
    let (client_read, client_write) = client.into_channels();
    let (backend_read, backend_write) = backend.into_channels();

    // Keep writers alive until both directions finish so a client EOF does not
    // close the adapter stdin before its response is forwarded.
    let client_write = Arc::new(Mutex::new(client_write));
    let backend_write = Arc::new(Mutex::new(backend_write));

    let client_to_backend = tokio::spawn(forward_shared(client_read, backend_write.clone()));
    let backend_to_client = tokio::spawn(forward_shared(backend_read, client_write.clone()));

    client_to_backend.await??;
    backend_to_client.await??;
    Ok(())
}

async fn forward_shared(
    mut read: ReadChannel,
    write: Arc<Mutex<WriteChannel>>,
) -> Result<(), ProtocolError> {
    while let Some(message) = read.recv().await? {
        let mut writer = write.lock().await;
        writer.send(&message).await?;
    }
    Ok(())
}

/// Default builtin plugin directory (workspace `plugins/builtin`).
pub fn default_plugins_dir() -> PathBuf {
    dap_plugin_api::default_builtin_dir()
}

/// Load plugin manifests from a directory of `*.yaml` files.
pub fn load_plugins_from_dir(dir: &Path) -> Result<Vec<dap_plugin_api::PluginManifest>> {
    dap_plugin_api::load_from_dir(dir).map_err(Into::into)
}
