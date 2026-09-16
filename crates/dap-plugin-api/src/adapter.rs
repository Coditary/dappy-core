use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterSpawn {
    pub transport: SpawnTransport,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SpawnTransport {
    Stdio,
    Tcp,
    /// In-process adapter provided by the proxy (`command` is the builtin id).
    Builtin,
}
