use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

use anyhow::{Context, Result};
use dap_protocol::{DuplexChannel, Message, ProtocolError, ReadChannel, WriteChannel};
use protocol_mux::{ClientEndpoint, ClientRole, MultiplexSession};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{debug, info};

use crate::adapter_guard::request_adapter_disconnect;
use crate::child_session::{
    ChildSessionConfig, ChildSessionSpawner, ParentSessionContext, handle_start_debugging,
};

/// Options for a multiplexed proxy session.
#[derive(Debug, Clone)]
pub struct MultiplexOptions {
    /// TCP port for control-plane attach (`0` = ephemeral). `None` disables the listener.
    pub control_port: Option<u16>,
    /// Close the editor client when no inbound DAP message arrives within this window.
    /// `None` disables the idle watchdog (default for long-lived IDE sessions).
    pub client_idle_timeout: Option<Duration>,
}

impl Default for MultiplexOptions {
    fn default() -> Self {
        Self {
            control_port: Some(0),
            client_idle_timeout: None,
        }
    }
}

/// Optional child-session handling for `startDebugging` reverse requests.
pub struct ChildSessionBridge {
    pub config: ChildSessionConfig,
    pub remaining_depth: u32,
    pub parent: ParentSessionContext,
    pub spawner: Arc<dyn ChildSessionSpawner>,
    pub active_children: Arc<AtomicU32>,
}

/// Runtime info returned after a multiplexed session ends.
#[derive(Debug, Clone)]
pub struct MultiplexedSessionInfo {
    pub control_port: Option<u16>,
}

/// Handle to a running multiplexed proxy task.
pub struct RunningMultiplexedProxy {
    pub control_port: Option<u16>,
    pub join: tokio::task::JoinHandle<Result<()>>,
}

/// Start a multiplexed proxy and return the bound control port before the editor disconnects.
pub async fn start_multiplexed_proxy(
    editor: DuplexChannel,
    backend: DuplexChannel,
    options: MultiplexOptions,
    child_sessions: Option<ChildSessionBridge>,
) -> Result<RunningMultiplexedProxy> {
    let (to_upstream, upstream_rx) = tokio::sync::mpsc::unbounded_channel();
    let (upstream_tx, from_upstream) = tokio::sync::mpsc::unbounded_channel();

    let (session, mux_task) = MultiplexSession::start(from_upstream, to_upstream);
    let backend_bridge = spawn_backend_bridge(
        backend,
        upstream_tx,
        upstream_rx,
        child_sessions.map(Arc::new),
    );

    let control_port = match options.control_port {
        Some(port) => Some(serve_control_attach(session.clone(), port).await?),
        None => None,
    };

    if let Some(port) = control_port {
        info!(control_port = port, "control attach listener ready");
    }

    let editor_endpoint = session.attach(ClientRole::Editor).await?;
    let session_for_editor = session.clone();

    let client_idle_timeout = options.client_idle_timeout;
    let join = tokio::spawn(async move {
        let result = run_client_bridge(
            editor,
            session_for_editor,
            editor_endpoint,
            ClientBridgeMode::Editor,
            client_idle_timeout,
        )
        .await;
        backend_bridge.shutdown().await;
        mux_task.abort();
        result
    });

    Ok(RunningMultiplexedProxy { control_port, join })
}

/// Start a multiplexed proxy with only a control attach listener (no editor client).
///
/// Used by headless REPL sessions that drive the adapter entirely via the control plane.
pub async fn start_headless_multiplexed_proxy(
    backend: DuplexChannel,
    options: MultiplexOptions,
    child_sessions: Option<ChildSessionBridge>,
) -> Result<RunningMultiplexedProxy> {
    let (to_upstream, upstream_rx) = tokio::sync::mpsc::unbounded_channel();
    let (upstream_tx, from_upstream) = tokio::sync::mpsc::unbounded_channel();

    let (session, mux_task) = MultiplexSession::start(from_upstream, to_upstream);
    let backend_bridge = spawn_backend_bridge(
        backend,
        upstream_tx,
        upstream_rx,
        child_sessions.map(Arc::new),
    );

    let control_port = match options.control_port {
        Some(port) => Some(serve_control_attach(session.clone(), port).await?),
        None => None,
    };

    if let Some(port) = control_port {
        info!(control_port = port, "headless control attach listener ready");
    }

    let join = tokio::spawn(async move {
        backend_bridge.shutdown().await;
        mux_task.abort();
        Ok(())
    });

    Ok(RunningMultiplexedProxy { control_port, join })
}

/// Run a multiplexed DAP proxy until the editor disconnects.
pub async fn run_multiplexed_proxy(
    editor: DuplexChannel,
    backend: DuplexChannel,
    options: MultiplexOptions,
    child_sessions: Option<ChildSessionBridge>,
) -> Result<MultiplexedSessionInfo> {
    let proxy = start_multiplexed_proxy(editor, backend, options, child_sessions).await?;
    let control_port = proxy.control_port;
    proxy.join.await??;
    Ok(MultiplexedSessionInfo { control_port })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClientBridgeMode {
    Editor,
    Control,
}

async fn run_client_bridge(
    duplex: DuplexChannel,
    session: MultiplexSession,
    endpoint: ClientEndpoint,
    mode: ClientBridgeMode,
    client_idle_timeout: Option<Duration>,
) -> Result<()> {
    let (mut read, write) = duplex.into_channels();
    let write = Arc::new(Mutex::new(write));
    let client_id = endpoint.id;
    let session_for_detach = session.clone();
    let idle_timeout = match mode {
        ClientBridgeMode::Editor => client_idle_timeout,
        ClientBridgeMode::Control => None,
    };

    let ingest = tokio::spawn(async move {
        loop {
            let message = match idle_timeout {
                Some(timeout) => match tokio::time::timeout(timeout, read.recv()).await {
                    Ok(Ok(Some(msg))) => msg,
                    Ok(Ok(None)) => break,
                    Ok(Err(err)) => return Err(err.into()),
                    Err(_) => {
                        info!(
                            client_id = ?client_id,
                            timeout_secs = timeout.as_secs(),
                            "editor client idle timeout; disconnecting"
                        );
                        break;
                    }
                },
                None => match read.recv().await? {
                    Some(msg) => msg,
                    None => break,
                },
            };
            session
                .send(client_id, message_to_value(&message)?)
                .map_err(|err| anyhow::anyhow!("mux client send: {err}"))?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let egress = tokio::spawn(async move {
        match mode {
            ClientBridgeMode::Editor | ClientBridgeMode::Control => {
                let mut inbound = endpoint.recv;
                while let Some(value) = inbound.recv().await {
                    let message = value_to_message(value)?;
                    write.lock().await.send(&message).await?;
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    });

    let ingest_result = ingest.await;
    let _ = session_for_detach.detach(client_id);
    let egress_result = egress.await;
    ingest_result??;
    egress_result??;
    Ok(())
}

struct BackendBridge {
    join: tokio::task::JoinHandle<()>,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl BackendBridge {
    async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), self.join).await;
    }
}

fn spawn_backend_bridge(
    backend: DuplexChannel,
    to_mux: tokio::sync::mpsc::UnboundedSender<Value>,
    from_mux: tokio::sync::mpsc::UnboundedReceiver<Value>,
    child_sessions: Option<Arc<ChildSessionBridge>>,
) -> BackendBridge {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let join = tokio::spawn(async move {
        if let Err(err) =
            bridge_backend(backend, to_mux, from_mux, child_sessions, shutdown_rx).await
        {
            debug!("backend mux bridge ended: {err}");
        }
    });
    BackendBridge {
        join,
        shutdown: shutdown_tx,
    }
}

async fn bridge_backend(
    backend: DuplexChannel,
    to_mux: tokio::sync::mpsc::UnboundedSender<Value>,
    mut from_mux: tokio::sync::mpsc::UnboundedReceiver<Value>,
    child_sessions: Option<Arc<ChildSessionBridge>>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<()> {
    let (mut backend_read, backend_write) = backend.into_channels();
    let backend_write = Arc::new(Mutex::new(backend_write));

    let backend_write_for_reverse = backend_write.clone();
    let child_sessions_for_bridge = child_sessions.clone();
    let backend_to_mux = tokio::spawn(async move {
        while let Some(message) = backend_read.recv().await? {
            if let Message::Request(request) = &message {
                if request.command == "startDebugging" {
                    if let Some(child) = child_sessions_for_bridge.as_ref() {
                        let response = handle_start_debugging(
                            request,
                            &child.config,
                            child.remaining_depth,
                            &child.parent,
                            child.spawner.as_ref(),
                            &child.active_children,
                        )
                        .await;
                        backend_write_for_reverse
                            .lock()
                            .await
                            .send(&response)
                            .await?;
                        continue;
                    }
                }
            }
            if let Some(response) = crate::reverse_request::handle_reverse_request(&message) {
                backend_write_for_reverse
                    .lock()
                    .await
                    .send(&response)
                    .await?;
                continue;
            }
            to_mux
                .send(message_to_value(&message)?)
                .map_err(|_| anyhow::anyhow!("mux upstream channel closed"))?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let mux_to_backend = tokio::spawn(async move {
        loop {
            tokio::select! {
                value = from_mux.recv() => {
                    match value {
                        Some(value) => {
                            let message = value_to_message(value)?;
                            backend_write.lock().await.send(&message).await?;
                        }
                        None => break,
                    }
                }
                _ = &mut shutdown_rx => {
                    break;
                }
            }
        }
        request_adapter_disconnect(&backend_write).await;
        Ok::<(), anyhow::Error>(())
    });

    backend_to_mux.await??;
    mux_to_backend.await??;
    Ok(())
}

async fn serve_control_attach(session: MultiplexSession, port: u16) -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("bind control attach port {port}"))?;
    let bound = listener.local_addr()?.port();

    tokio::spawn(async move {
        while let Ok((stream, _addr)) = listener.accept().await {
            let session = session.clone();
            tokio::spawn(async move {
                let (reader, writer) = stream.into_split();
                let duplex = DuplexChannel::from_streams(reader, writer);
                match session.attach(ClientRole::Control).await {
                    Ok(endpoint) => {
                        let id = endpoint.id;
                        let result = run_client_bridge(
                            duplex,
                            session.clone(),
                            endpoint,
                            ClientBridgeMode::Control,
                            None,
                        )
                        .await;
                        let _ = session.detach(id);
                        if let Err(err) = result {
                            debug!("control client ended with error: {err}");
                        }
                    }
                    Err(err) => debug!("control attach failed: {err}"),
                }
            });
        }
    });

    Ok(bound)
}

fn message_to_value(message: &Message) -> Result<Value> {
    serde_json::to_value(message).context("serialize DAP message")
}

fn value_to_message(value: Value) -> Result<Message, ProtocolError> {
    serde_json::from_value(value).map_err(ProtocolError::Json)
}

/// Connect a control client to a running session over TCP (for CLI / agents).
pub async fn connect_control_client(port: u16) -> Result<DuplexChannel> {
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .with_context(|| format!("connect control attach port {port}"))?;
    let (reader, writer) = stream.into_split();
    Ok(DuplexChannel::from_streams(reader, writer))
}

/// Send one DAP request and read until a matching response arrives.
pub async fn roundtrip_request(
    read: &mut ReadChannel,
    write: &mut WriteChannel,
    seq: i64,
    command: &str,
    arguments: Option<Value>,
) -> Result<Message> {
    let request = Message::Request(dap_protocol::Request {
        seq,
        command: command.to_string(),
        arguments,
    });
    write.send(&request).await?;
    while let Some(message) = read.recv().await? {
        if let Message::Response(response) = &message {
            if response.request_seq == seq {
                return Ok(message);
            }
        }
    }
    anyhow::bail!("channel closed before response to {command}")
}
