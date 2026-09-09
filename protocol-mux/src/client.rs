use std::fmt::Debug;
use std::hash::Hash;

/// Client identifier in a multiplexed session.
pub trait ClientId: Copy + Eq + Hash + Debug + Send + Sync + 'static {
    fn from_u64(value: u64) -> Self;
}

/// Role of an attached client (for logging, policy, and routing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientRole {
    /// Primary editor / IDE connection (stdio).
    Editor,
    /// Control-plane CLI attaching to a live session.
    Control,
    /// Automated agent (MCP, scripts).
    Agent,
    /// Other / extension point.
    Other,
}

/// Default concrete client id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Id(u64);

impl ClientId for Id {
    fn from_u64(value: u64) -> Self {
        Self(value)
    }
}

impl Id {
    pub fn raw(self) -> u64 {
        self.0
    }
}
