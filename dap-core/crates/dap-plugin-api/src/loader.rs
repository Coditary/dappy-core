use std::path::{Path, PathBuf};

use crate::error::PluginError;
use crate::manifest::PluginManifest;
use crate::target::TargetManifest;

/// Default builtin plugin directory (sibling `dap-plugins/builtin` project).
pub fn default_builtin_dir() -> PathBuf {
    std::env::var("DAP_PLUGINS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../dap-plugins/builtin")
        })
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
    let mut manifest: PluginManifest =
        serde_yaml::from_str(&text).map_err(|source| PluginError::Parse {
            path: path.display().to_string(),
            source,
        })?;
    manifest.source_path = Some(path.to_path_buf());
    manifest.validate()?;
    Ok(manifest)
}

/// Optional user target directory (`$XDG_CONFIG_HOME/dap/targets` or `~/.config/dap/targets`).
pub fn user_targets_dir() -> Option<PathBuf> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok()?;
    Some(base.join("dap/targets"))
}

/// Load and validate an RSP target manifest from a YAML file.
pub fn load_target_from_file(path: &Path) -> Result<TargetManifest, PluginError> {
    let text = std::fs::read_to_string(path).map_err(|source| PluginError::Io {
        path: path.display().to_string(),
        source,
    })?;
    TargetManifest::from_yaml(&text, path)
}

/// Load a companion attachment YAML referenced by a plugin manifest.
pub fn load_attachment(path: &Path) -> Result<serde_yaml::Value, PluginError> {
    let text = std::fs::read_to_string(path).map_err(|source| PluginError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_yaml::from_str(&text).map_err(|source| PluginError::Parse {
        path: path.display().to_string(),
        source,
    })
}

/// Filenames recognized as a plugin manifest inside a per-plugin directory.
pub const PLUGIN_MANIFEST_NAMES: &[&str] = &[
    "plugin.yaml",
    "plugin.yml",
    "manifest.yaml",
    "manifest.yml",
];

/// Filenames recognized as an RSP target manifest inside a per-target directory.
pub const TARGET_MANIFEST_NAMES: &[&str] = &[
    "target.yaml",
    "target.yml",
    "manifest.yaml",
    "manifest.yml",
];

/// Resolve the manifest file inside a per-plugin directory, if present.
pub fn manifest_in_plugin_dir(dir: &Path) -> Option<PathBuf> {
    for name in PLUGIN_MANIFEST_NAMES {
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Resolve the target manifest file inside a per-target directory, if present.
pub fn target_manifest_in_dir(dir: &Path) -> Option<PathBuf> {
    for name in TARGET_MANIFEST_NAMES {
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Load all plugin manifests from a directory.
///
/// Supports:
/// - per-plugin folders: `builtin/python/plugin.yaml`
/// - flat manifests (user overrides): `~/.config/dap/plugins/custom.yaml`
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
        if path.is_dir() {
            if let Some(manifest_path) = manifest_in_plugin_dir(&path) {
                manifests.push(load_from_file(&manifest_path)?);
            }
            continue;
        }
        if is_plugin_manifest_file(&path) {
            manifests.push(load_from_file(&path)?);
        }
    }
    manifests.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(manifests)
}

fn is_plugin_manifest_file(path: &Path) -> bool {
    if !path
        .extension()
        .is_some_and(|ext| ext == "yaml" || ext == "yml")
    {
        return false;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return false,
    };
    let value: serde_yaml::Value = match serde_yaml::from_str(&text) {
        Ok(value) => value,
        Err(_) => return false,
    };
    value.get("adapter").is_some() && value.get("id").is_some()
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

/// Load all RSP target manifests from a directory.
pub fn load_targets_from_dir(dir: &Path) -> Result<Vec<TargetManifest>, PluginError> {
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
        if path.is_dir() {
            if let Some(manifest_path) = target_manifest_in_dir(&path) {
                manifests.push(load_target_from_file(&manifest_path)?);
            }
            continue;
        }
        if is_target_manifest_file(&path) {
            manifests.push(load_target_from_file(&path)?);
        }
    }
    manifests.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(manifests)
}

fn is_target_manifest_file(path: &Path) -> bool {
    if !path
        .extension()
        .is_some_and(|ext| ext == "yaml" || ext == "yml")
    {
        return false;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return false,
    };
    let value: serde_yaml::Value = match serde_yaml::from_str(&text) {
        Ok(value) => value,
        Err(_) => return false,
    };
    value.get("targetTypes").is_some() && value.get("id").is_some() && value.get("adapter").is_none()
}

/// Load builtin and user RSP target manifests (user entries override by id).
pub fn load_target_defaults() -> Result<Vec<TargetManifest>, PluginError> {
    let mut by_id = std::collections::BTreeMap::new();
    for manifest in load_targets_from_dir(&default_builtin_dir())? {
        by_id.insert(manifest.id.clone(), manifest);
    }
    if let Some(user_dir) = user_targets_dir() {
        for manifest in load_targets_from_dir(&user_dir)? {
            by_id.insert(manifest.id.clone(), manifest);
        }
    }
    Ok(by_id.into_values().collect())
}
