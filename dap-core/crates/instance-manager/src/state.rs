use serde::{Deserialize, Serialize};

/// Lifecycle state of a managed instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    Pending,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
}

impl InstanceState {
    pub fn can_transition_to(&self, next: InstanceState) -> bool {
        use InstanceState::*;
        matches!(
            (self, next),
            (Pending, Starting)
                | (Starting, Running)
                | (Starting, Failed)
                | (Running, Stopping)
                | (Running, Failed)
                | (Stopping, Stopped)
                | (Stopping, Failed)
                | (Stopped, Starting)
                | (Failed, Starting)
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_matrix_allows_restart_from_stopped() {
        assert!(InstanceState::Stopped.can_transition_to(InstanceState::Starting));
        assert!(!InstanceState::Pending.can_transition_to(InstanceState::Running));
        assert_eq!(InstanceState::Running.label(), "running");
    }
}
