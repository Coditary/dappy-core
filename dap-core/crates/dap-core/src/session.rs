use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Options extracted from a client launch/attach request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchOptions {
    pub request: String,
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub adapter: Option<String>,
    #[serde(flatten)]
    pub extra: Value,
}

/// Handle to a debug session tracked by the engine.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionHandle {
    pub instance_id: instance_manager::InstanceId,
    pub adapter_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_port: Option<u16>,
}
