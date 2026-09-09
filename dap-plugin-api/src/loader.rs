use std::path::{Path, PathBuf};

use crate::error::PluginError;
use crate::manifest::PluginManifest;

/// Default builtin plugin directory (workspace `plugins/builtin`).
pub fn default_builtin_dir() -> PathBuf {
    std::env::var("DAP_PLUGINS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins/builtin"))
}

/// Optional user plugin directory (`$XDG_CONFIG_HOME/dap/plugins` or `~/.config/dap/plugins`).
pub fn user_plugins_dir() -> Option<PathBuf> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok()?;
    Some(base.join("dap/plugins"))
}

/// Load and validate a plugin manifest from a YAML file.
pub fn load_from_file(path: &Path) -> Result<PluginManifest, PluginError> {
    let text = std::fs::read_to_string(path).map_err(|source| PluginError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let manifest: PluginManifest = serde_yaml::from_str(&text).map_err(|source| PluginError::Parse {
        path: path.display().to_string(),
        source,
    })?;
    manifest.validate()?;
    Ok(manifest)
}

/// Load all `*.yaml` / `*.yml` manifests from a directory.
pub fn load_from_dir(dir: &Path) -> Result<Vec<PluginManifest>, PluginError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut manifests = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|source| PluginError::Io {
        path: dir.display().to_string(),
        source,
    })? {
        let path = entry
            .map_err(|source| PluginError::Io {
                path: dir.display().to_string(),
                source,
            })?
            .path();
        if path
            .extension()
            .is_some_and(|ext| ext == "yaml" || ext == "yml")
        {
            manifests.push(load_from_file(&path)?);
        }
    }
    manifests.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(manifests)
}

/// Load builtin and user plugin manifests (user entries override by id).
pub fn load_defaults() -> Result<Vec<PluginManifest>, PluginError> {
    let mut by_id = std::collections::BTreeMap::new();
    for manifest in load_from_dir(&default_builtin_dir())? {
        by_id.insert(manifest.id.clone(), manifest);
    }
    if let Some(user_dir) = user_plugins_dir() {
        for manifest in load_from_dir(&user_dir)? {
            by_id.insert(manifest.id.clone(), manifest);
        }
    }
    Ok(by_id.into_values().collect())
}
