use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::cache::LateJoinCache;
use crate::cancel::translate_cancel;
use crate::client::{ClientId, ClientRole, Id};
use crate::error::MuxError;
use crate::message::{MessageKind, command, message_kind, request_seq, response, seq, set_request_seq, set_seq};
use crate::remapper::{BackendSeq, ClientSeq, MessageRemapper};

const DEFAULT_BROADCAST_CAPACITY: usize = 256;

fn uses_seq_remapping(role: ClientRole) -> bool {
    role == ClientRole::Editor
}

/// Handle to a running multiplex session.
#[derive(Clone)]
pub struct MultiplexSession<C: ClientId = Id> {
    ops: mpsc::UnboundedSender<SessionOp<C>>,
}

enum SessionOp<C: ClientId> {
    Attach {
        role: ClientRole,
        reply: oneshot::Sender<ClientEndpoint<C>>,
    },
    ClientSend {
        id: C,
        message: Value,
    },
    Detach {
        id: C,
    },
}

/// One attached client endpoint.
pub struct ClientEndpoint<C: ClientId = Id> {
    pub id: C,
    pub role: ClientRole,
    session_ops: mpsc::UnboundedSender<SessionOp<C>>,
    pub recv: mpsc::UnboundedReceiver<Value>,
    pub events: broadcast::Receiver<Arc<Value>>,
}

impl<C: ClientId> ClientEndpoint<C> {
    pub fn send(&self, message: Value) -> Result<(), MuxError> {
        self.session_ops
            .send(SessionOp::ClientSend {
                id: self.id,
                message,
            })
            .map_err(|_| MuxError::ClientNotFound)
    }

    pub async fn recv_message(&mut self) -> Option<Value> {
        self.recv.recv().await
    }

    pub async fn recv_event(&mut self) -> Option<Arc<Value>> {
        self.events.recv().await.ok()
    }
}

struct ClientState {
    role: ClientRole,
    outbound: mpsc::UnboundedSender<Value>,
}

struct SessionLoop<C: ClientId> {
    from_upstream: mpsc::UnboundedReceiver<Value>,
    to_upstream: mpsc::UnboundedSender<Value>,
    ops_tx: mpsc::UnboundedSender<SessionOp<C>>,
    ops: mpsc::UnboundedReceiver<SessionOp<C>>,
    clients: HashMap<C, ClientState>,
    secondary_pending: HashMap<i64, (C, i64)>,
    remapper: MessageRemapper,
    backend_seq: AtomicI64,
    display_seq: AtomicI64,
    broadcast: broadcast::Sender<Arc<Value>>,
    cache: LateJoinCache,
    next_client_id: u64,
}

impl MultiplexSession<Id> {
    /// Start the multiplex loop on a background task (default client id type).
    pub fn start(
        from_upstream: mpsc::UnboundedReceiver<Value>,
        to_upstream: mpsc::UnboundedSender<Value>,
    ) -> (Self, JoinHandle<()>) {
        Self::start_with_capacity(from_upstream, to_upstream, DEFAULT_BROADCAST_CAPACITY)
    }

    pub fn start_with_capacity(
        from_upstream: mpsc::UnboundedReceiver<Value>,
        to_upstream: mpsc::UnboundedSender<Value>,
        broadcast_capacity: usize,
    ) -> (Self, JoinHandle<()>) {
        start_session(from_upstream, to_upstream, broadcast_capacity)
    }
}

impl<C: ClientId> MultiplexSession<C> {
    pub async fn attach(&self, role: ClientRole) -> Result<ClientEndpoint<C>, MuxError> {
        let (reply, rx) = oneshot::channel();
        self.ops
            .send(SessionOp::Attach { role, reply })
            .map_err(|_| MuxError::Other("session loop stopped".into()))?;
        rx.await
            .map_err(|_| MuxError::Other("session loop stopped".into()))
    }

    pub fn send(&self, id: C, message: Value) -> Result<(), MuxError> {
        self.ops
            .send(SessionOp::ClientSend { id, message })
            .map_err(|_| MuxError::ClientNotFound)
    }

    pub fn detach(&self, id: C) -> Result<(), MuxError> {
        self.ops
            .send(SessionOp::Detach { id })
            .map_err(|_| MuxError::ClientNotFound)
    }
}

fn start_session<C: ClientId>(
    from_upstream: mpsc::UnboundedReceiver<Value>,
    to_upstream: mpsc::UnboundedSender<Value>,
    broadcast_capacity: usize,
) -> (MultiplexSession<C>, JoinHandle<()>) {
    let (ops_tx, ops_rx) = mpsc::unbounded_channel();
    let (broadcast_tx, _) = broadcast::channel(broadcast_capacity);

    let loop_state = SessionLoop {
        from_upstream,
        to_upstream,
        ops_tx: ops_tx.clone(),
        ops: ops_rx,
        clients: HashMap::new(),
        secondary_pending: HashMap::new(),
        remapper: MessageRemapper::new(),
        backend_seq: AtomicI64::new(0),
        display_seq: AtomicI64::new(0),
        broadcast: broadcast_tx.clone(),
        cache: LateJoinCache::default(),
        next_client_id: 0,
    };

    let handle = tokio::spawn(async move {
        if let Err(err) = loop_state.run().await {
            warn!("multiplex session ended with error: {err}");
        }
    });

    (MultiplexSession { ops: ops_tx }, handle)
}

impl<C: ClientId> SessionLoop<C> {
    async fn run(mut self) -> Result<(), MuxError> {
        loop {
            tokio::select! {
                message = self.from_upstream.recv() => {
                    match message {
                        Some(msg) => self.handle_upstream_message(msg)?,
                        None => break,
                    }
                }
                op = self.ops.recv() => {
                    match op {
                        Some(op) => self.handle_op(op)?,
                        None => break,
                    }
                }
            }
        }
        Ok(())
    }

    fn handle_op(&mut self, op: SessionOp<C>) -> Result<(), MuxError> {
        match op {
            SessionOp::Attach { role, reply } => {
                self.next_client_id += 1;
                let id = C::from_u64(self.next_client_id);
                let (client_tx, client_rx) = mpsc::unbounded_channel();
                let events = self.broadcast.subscribe();

                self.clients.insert(
                    id,
                    ClientState {
                        role,
                        outbound: client_tx,
                    },
                );

                for cached in self.cache.replay() {
                    let _ = self.deliver_to_client(id, cached);
                }

                let _ = reply.send(ClientEndpoint {
                    id,
                    role,
                    session_ops: self.ops_tx.clone(),
                    recv: client_rx,
                    events,
                });
            }
            SessionOp::ClientSend { id, message } => {
                self.handle_client_message(id, message)?;
            }
            SessionOp::Detach { id } => {
                self.clients.remove(&id);
            }
        }
        Ok(())
    }

    fn handle_client_message(&mut self, id: C, mut message: Value) -> Result<(), MuxError> {
        let role = self
            .clients
            .get(&id)
            .map(|state| state.role)
            .ok_or(MuxError::ClientNotFound)?;

        if message_kind(&message) != MessageKind::Request {
            warn!("ignoring non-request from client {id:?}");
            return Ok(());
        }

        if role == ClientRole::Control && command(&message) == Some(BREAKPOINT_SNAPSHOT_COMMAND) {
            let client_seq = seq(&message).unwrap_or(0);
            let body = self.cache.breakpoint_snapshot();
            let outbound = response(self.next_display_seq(), client_seq, true, Some(body));
            self.deliver_to_client(id, outbound)?;
            return Ok(());
        }

        let client_seq = seq(&message).map(ClientSeq::new);

        if uses_seq_remapping(role) {
            if let Some(client_seq) = client_seq {
                translate_cancel(&mut message, &self.remapper);
                let backend = BackendSeq::new(self.next_backend_seq());
                set_seq(&mut message, backend.raw());
                self.remapper.map(client_seq, backend);
                self.cache.observe_client_request(backend.raw(), &message);
            }
        } else {
            let backend = self.next_backend_seq();
            let client_seq_raw = client_seq.map(|s| s.raw()).unwrap_or(backend);
            set_seq(&mut message, backend);
            self.secondary_pending.insert(backend, (id, client_seq_raw));
            self.cache.observe_client_request(backend, &message);
        }

        debug!(client = ?id, role = ?role, "forwarding client request upstream");
        self.to_upstream
            .send(message)
            .map_err(|_| MuxError::UpstreamNotConnected)?;
        Ok(())
    }

    fn handle_upstream_message(&mut self, mut message: Value) -> Result<(), MuxError> {
        self.publish(&message);

        match message_kind(&message) {
            MessageKind::Response => {
                self.cache.observe_response(&message);
                let backend_request_seq = request_seq(&message)
                    .ok_or_else(|| MuxError::Other("response missing requestSeq".into()))?;
                let backend = BackendSeq::new(backend_request_seq);

                if let Some(client_seq) = self.remapper.unmap(backend) {
                    set_request_seq(&mut message, client_seq.raw());
                    self.deliver_to_editor(&message)?;
                } else if let Some((client_id, client_seq)) =
                    self.secondary_pending.remove(&backend_request_seq)
                {
                    set_request_seq(&mut message, client_seq);
                    self.deliver_to_client(client_id, message)?;
                } else {
                    debug!(backend_request_seq, "dropping unmatched upstream response");
                }
            }
            MessageKind::Event => {
                self.cache.observe(&message);
                self.broadcast_to_all(&message)?;
            }
            MessageKind::Request | MessageKind::Unknown => {
                warn!("ignoring unexpected upstream message type");
            }
        }

        Ok(())
    }

    fn publish(&self, message: &Value) {
        if self.broadcast.receiver_count() > 0 {
            let _ = self.broadcast.send(Arc::new(message.clone()));
        }
    }

    fn broadcast_to_all(&mut self, message: &Value) -> Result<(), MuxError> {
        let display = self.next_display_seq();
        let mut outbound = message.clone();
        set_seq(&mut outbound, display);
        self.publish(&outbound);

        for (id, state) in &self.clients {
            if matches!(state.role, ClientRole::Editor | ClientRole::Control) {
                self.deliver_to_client(*id, outbound.clone())?;
            }
        }
        Ok(())
    }

    fn deliver_to_editor(&mut self, message: &Value) -> Result<(), MuxError> {
        let display = self.next_display_seq();
        let mut outbound = message.clone();
        set_seq(&mut outbound, display);
        self.deliver_to_role(ClientRole::Editor, outbound)
    }

    fn deliver_to_role(&self, role: ClientRole, message: Value) -> Result<(), MuxError> {
        for (id, state) in &self.clients {
            if state.role == role {
                self.deliver_to_client(*id, message.clone())?;
            }
        }
        Ok(())
    }

    fn deliver_to_client(&self, id: C, message: Value) -> Result<(), MuxError> {
        let outbound = self
            .clients
            .get(&id)
            .map(|state| &state.outbound)
            .ok_or(MuxError::ClientNotFound)?;
        outbound
            .send(message)
            .map_err(|_| MuxError::ClientNotFound)?;
        Ok(())
    }

    fn next_backend_seq(&self) -> i64 {
        self.backend_seq.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn next_display_seq(&self) -> i64 {
        self.display_seq.fetch_add(1, Ordering::SeqCst) + 1
    }
}

/// Control-plane request served locally by the multiplexer (not forwarded upstream).
pub const BREAKPOINT_SNAPSHOT_COMMAND: &str = "_dapBreakpointSnapshot";

/// Legacy registry wrapper kept for lightweight bookkeeping in tests and callers
/// that only need client ids without the async loop.
#[derive(Debug)]
pub struct Multiplexer<C: ClientId> {
    pub remapper: MessageRemapper,
    next_client_id: u64,
    clients: HashMap<C, ClientRole>,
}

impl<C: ClientId> Default for Multiplexer<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: ClientId> Multiplexer<C> {
    pub fn new() -> Self {
        Self {
            remapper: MessageRemapper::new(),
            next_client_id: 0,
            clients: HashMap::new(),
        }
    }

    pub fn attach_client(&mut self, role: ClientRole) -> C {
        self.next_client_id += 1;
        let id = C::from_u64(self.next_client_id);
        self.clients.insert(id, role);
        id
    }

    pub fn detach_client(&mut self, id: C) -> Option<ClientRole> {
        self.clients.remove(&id)
    }

    pub fn client_count(&self) -> usize {
        self.clients.len()
    }
}
