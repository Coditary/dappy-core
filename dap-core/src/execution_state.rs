use dap_protocol::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// High-level debuggee execution status derived from DAP events.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    #[default]
    Unknown,
    Running,
    Stopped,
    Exited,
}

/// Snapshot of execution state for CLI / MCP `status`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionStateSummary {
    pub status: ExecutionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Monotonic version wrapper for change detection.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionedExecutionState {
    pub version: u64,
    #[serde(flatten)]
    pub state: ExecutionStateSummary,
}

/// Tracks execution state from observed DAP messages.
#[derive(Debug, Clone, Default)]
pub struct ExecutionStateTracker {
    version: u64,
    state: ExecutionStateSummary,
}

impl ExecutionStateTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn summary(&self) -> VersionedExecutionState {
        VersionedExecutionState {
            version: self.version,
            state: self.state.clone(),
        }
    }

    pub fn apply_message(&mut self, message: &Message) {
        if let Message::Event(event) = message {
            self.apply_event(&event.event, event.body.as_ref());
        }
    }

    pub fn apply_event(&mut self, name: &str, body: Option<&Value>) {
        self.version += 1;
        match name {
            "stopped" => {
                self.state.status = ExecutionStatus::Stopped;
                self.state.thread_id = body
                    .and_then(|b| b.get("threadId"))
                    .and_then(|v| v.as_i64());
                self.state.stop_reason = body
                    .and_then(|b| b.get("reason"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                self.state.description = body
                    .and_then(|b| b.get("text"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
            }
            "continued" => {
                self.state.status = ExecutionStatus::Running;
                self.state.stop_reason = None;
                self.state.description = None;
            }
            "exited" | "terminated" => {
                self.state.status = ExecutionStatus::Exited;
                self.state.stop_reason = body
                    .and_then(|b| b.get("reason"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
            }
            "initialized" => {
                self.state.status = ExecutionStatus::Running;
            }
            _ => {}
        }
    }
}
