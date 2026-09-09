use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::id::InstanceId;

/// Optional control-plane listen port (CLI / agent attach).
pub type ControlPort = u16;

/// Metadata supplied when registering a new instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceSpec {
    pub id: InstanceId,
    pub kind: String,
    pub label: Option<String>,
    pub scope: Option<String>,
    /// Control-plane port for CLI/agent attach (when applicable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_port: Option<ControlPort>,
    /// Parent instance when this was spawned as a child (e.g. `startDebugging`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<InstanceId>,
    /// Owning process id when the instance is backed by a live child process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default)]
    pub tags: HashMap<String, String>,
    pub created_at_unix: i64,
}

impl InstanceSpec {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            id: InstanceId::generate(),
            kind: kind.into(),
            label: None,
            scope: None,
            control_port: None,
            parent_id: None,
            pid: None,
            tags: HashMap::new(),
            created_at_unix: now_unix(),
        }
    }

    pub fn with_id(mut self, id: InstanceId) -> Self {
        self.id = id;
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    pub fn with_control_port(mut self, port: ControlPort) -> Self {
        self.control_port = Some(port);
        self
    }

    pub fn with_parent_id(mut self, parent: InstanceId) -> Self {
        self.parent_id = Some(parent);
        self
    }

    pub fn with_pid(mut self, pid: u32) -> Self {
        self.pid = Some(pid);
        self
    }

    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_sets_metadata_fields() {
        let parent = InstanceId::generate();
        let spec = InstanceSpec::new("debug")
            .with_label("session-a")
            .with_scope("workspace")
            .with_control_port(4711)
            .with_parent_id(parent.clone())
            .with_tag("env", "test");
        assert_eq!(spec.label.as_deref(), Some("session-a"));
        assert_eq!(spec.scope.as_deref(), Some("workspace"));
        assert_eq!(spec.control_port, Some(4711));
        assert_eq!(spec.parent_id, Some(parent));
        assert_eq!(spec.tags.get("env").map(String::as_str), Some("test"));
    }
}
