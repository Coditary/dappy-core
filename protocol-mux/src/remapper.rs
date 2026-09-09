use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Opaque sequence number from a downstream client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientSeq(i64);

/// Opaque sequence number on the upstream connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BackendSeq(i64);

impl ClientSeq {
    pub fn new(value: i64) -> Self {
        Self(value)
    }

    pub fn raw(self) -> i64 {
        self.0
    }
}

impl BackendSeq {
    pub fn new(value: i64) -> Self {
        Self(value)
    }

    pub fn raw(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Default)]
struct RemapperInner {
    forward: HashMap<ClientSeq, BackendSeq>,
    reverse: HashMap<BackendSeq, ClientSeq>,
}

/// Maps client request sequence numbers to upstream sequence numbers.
///
/// Ported from the design in Meta Dapper's `MessageRemapper` (MIT).
#[derive(Debug, Clone, Default)]
pub struct MessageRemapper {
    inner: Arc<Mutex<RemapperInner>>,
}

impl MessageRemapper {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn map(&self, client_seq: ClientSeq, backend_seq: BackendSeq) {
        let mut inner = self.inner.lock().expect("remapper mutex poisoned");
        inner.forward.insert(client_seq, backend_seq);
        inner.reverse.insert(backend_seq, client_seq);
    }

    pub fn unmap(&self, backend_seq: BackendSeq) -> Option<ClientSeq> {
        let mut inner = self.inner.lock().expect("remapper mutex poisoned");
        let client_seq = inner.reverse.remove(&backend_seq)?;
        inner.forward.remove(&client_seq);
        Some(client_seq)
    }

    pub fn lookup_backend(&self, client_seq: ClientSeq) -> Option<BackendSeq> {
        let inner = self.inner.lock().expect("remapper mutex poisoned");
        inner.forward.get(&client_seq).copied()
    }

    pub fn lookup_client(&self, backend_seq: BackendSeq) -> Option<ClientSeq> {
        let inner = self.inner.lock().expect("remapper mutex poisoned");
        inner.reverse.get(&backend_seq).copied()
    }
}
