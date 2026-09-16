use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::adapter::AdapterSpawn;
use crate::error::PluginError;
use crate::launch::{InitSpec, InitializeSpec, LaunchSpec};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initialize: Option<InitializeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<LaunchSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init: Option<InitSpec>,
    /// Optional client-side companion YAML (relative to this manifest file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<String>,
    /// Manifest file path (set by the loader, not serialized).
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
}

impl PluginManifest {
    /// Resolved absolute path to the companion attachment YAML, if configured.
    pub fn attachment_path(&self) -> Option<PathBuf> {
        sidecar_path(self.attachment.as_deref(), self.source_path.as_deref())
    }

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
        if let Some(attachment) = &self.attachment {
            validate_sidecar_reference("attachment", attachment, self.source_path.as_deref())?;
        }
        Ok(())
    }
}

fn sidecar_path(relative: Option<&str>, source_path: Option<&Path>) -> Option<PathBuf> {
    let relative = relative?;
    let manifest_path = source_path?;
    let parent = manifest_path.parent()?;
    let path = parent.join(relative);
    Some(path.canonicalize().unwrap_or(path))
}

fn validate_sidecar_reference(
    field: &str,
    relative: &str,
    source_path: Option<&Path>,
) -> Result<(), PluginError> {
    let trimmed = relative.trim();
    if trimmed.is_empty() {
        return Err(PluginError::InvalidManifest(format!("{field} must not be empty")));
    }
    if !(trimmed.ends_with(".yaml") || trimmed.ends_with(".yml")) {
        return Err(PluginError::InvalidManifest(format!(
            "{field} must point to a .yaml or .yml file"
        )));
    }
    if let Some(source_path) = source_path {
        let parent = source_path
            .parent()
            .ok_or_else(|| PluginError::InvalidManifest("manifest has no parent directory".into()))?;
        let path = parent.join(trimmed);
        if !path.is_file() {
            return Err(PluginError::InvalidManifest(format!(
                "{field} file not found: {}",
                path.display()
            )));
        }
        let text = std::fs::read_to_string(&path).map_err(|source| PluginError::Io {
            path: path.display().to_string(),
            source,
        })?;
        serde_yaml::from_str::<serde_yaml::Value>(&text).map_err(|source| PluginError::Parse {
            path: path.display().to_string(),
            source,
        })?;
    }
    Ok(())
}
