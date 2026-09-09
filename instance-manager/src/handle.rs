use serde::{Deserialize, Serialize};

use crate::id::InstanceId;
use crate::spec::InstanceSpec;
use crate::state::InstanceState;
use crate::store::{is_control_port_in_use, is_pid_alive};

/// Snapshot of a managed instance exposed to callers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceHandle {
    pub spec: InstanceSpec,
    pub state: InstanceState,
}

impl InstanceHandle {
    pub fn id(&self) -> &InstanceId {
        &self.spec.id
    }

    pub fn kind(&self) -> &str {
        &self.spec.kind
    }

    pub fn state(&self) -> InstanceState {
        self.state
    }

    /// Whether the owning process and control port (when set) still look alive.
    pub fn is_alive(&self) -> bool {
        match self.state {
            InstanceState::Stopped | InstanceState::Failed => return false,
            _ => {}
        }
        if let Some(pid) = self.spec.pid {
            if !is_pid_alive(pid) {
                return false;
            }
        }
        if let Some(port) = self.spec.control_port {
            if !is_control_port_in_use(port) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::InstanceSpec;

    #[test]
    fn handle_exposes_spec_fields() {
        let handle = InstanceHandle {
            spec: InstanceSpec::new("dap-session").with_label("main.py"),
            state: InstanceState::Running,
        };
        assert_eq!(handle.kind(), "dap-session");
        assert_eq!(handle.state(), InstanceState::Running);
        assert!(!handle.id().as_str().is_empty());
    }

    #[test]
    fn failed_instances_are_not_alive() {
        let handle = InstanceHandle {
            spec: InstanceSpec::new("dap-session"),
            state: InstanceState::Failed,
        };
        assert!(!handle.is_alive());
    }
}
