use serde::{Deserialize, Serialize};

use crate::adapter::AdapterSpawn;
use crate::error::PluginError;

/// Declarative plugin metadata (YAML/JSON on disk).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default, rename = "launchTypes")]
    pub launch_types: Vec<String>,
    #[serde(default, rename = "fileExtensions")]
    pub file_extensions: Vec<String>,
    pub adapter: AdapterSpawn,
}

impl PluginManifest {
    /// Validate required fields and routing metadata.
    pub fn validate(&self) -> Result<(), PluginError> {
        if self.id.trim().is_empty() {
            return Err(PluginError::InvalidManifest("id must not be empty".into()));
        }
        if self.name.trim().is_empty() {
            return Err(PluginError::InvalidManifest("name must not be empty".into()));
        }
        if self.version.trim().is_empty() {
            return Err(PluginError::InvalidManifest("version must not be empty".into()));
        }
        if self.adapter.command.trim().is_empty() {
            return Err(PluginError::InvalidManifest(
                "adapter.command must not be empty".into(),
            ));
        }
        if self.launch_types.is_empty() && self.file_extensions.is_empty() {
            return Err(PluginError::InvalidManifest(
                "at least one launchType or fileExtension is required".into(),
            ));
        }
        Ok(())
    }
}
