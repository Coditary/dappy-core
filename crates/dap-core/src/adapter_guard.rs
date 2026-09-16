use std::time::Duration;

use dap_protocol::{Message, Request, WriteChannel};
use serde_json::json;
use tokio::sync::Mutex;

use crate::proxy::{Backend, kill_adapter_process};

/// Keeps the adapter subprocess handle alive until explicit shutdown.
pub struct AdapterGuard {
    process: Option<tokio::process::Child>,
}

impl AdapterGuard {
    pub fn from_backend(backend: Backend) -> (dap_protocol::DuplexChannel, Self) {
        let (duplex, process) = backend.detach_adapter();
        (duplex, Self { process })
    }

    pub fn empty() -> Self {
        Self { process: None }
    }

    /// Best-effort graceful `disconnect`, then kill and reap the adapter child.
    pub async fn shutdown(&mut self) {
        kill_adapter_process(&mut self.process);
        if let Some(mut child) = self.process.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
        }
    }
}

/// Send `disconnect` with `terminateDebuggee` before tearing down the adapter pipe.
pub async fn request_adapter_disconnect(write: &Mutex<WriteChannel>) {
    let request = Message::Request(Request {
        seq: 1,
        command: "disconnect".into(),
        arguments: Some(json!({ "terminateDebuggee": true })),
    });
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        let mut writer = write.lock().await;
        writer.send(&request).await
    })
    .await;
}
