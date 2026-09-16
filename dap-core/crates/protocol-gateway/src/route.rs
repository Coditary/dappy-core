use serde::{Deserialize, Serialize};

/// Inputs available when resolving a route (protocol-agnostic).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouteContext {
    /// Explicit plugin id from user/config (highest priority).
    pub explicit_plugin: Option<String>,
    /// Protocol-specific launch `type` field (e.g. DAP `debugpy`).
    pub launch_type: Option<String>,
    /// Primary target path (source file, binary, workspace root, …).
    pub program_path: Option<String>,
    /// Optional scope for multi-tenant routing.
    pub scope: Option<String>,
}

impl RouteContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_explicit_plugin(mut self, id: impl Into<String>) -> Self {
        self.explicit_plugin = Some(id.into());
        self
    }

    pub fn with_launch_type(mut self, t: impl Into<String>) -> Self {
        self.launch_type = Some(t.into());
        self
    }

    pub fn with_program_path(mut self, path: impl Into<String>) -> Self {
        self.program_path = Some(path.into());
        self
    }
}

/// Result of a successful route resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteMatch {
    pub plugin_id: String,
}
