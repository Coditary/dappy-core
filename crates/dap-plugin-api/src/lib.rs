//! Plugin manifest types, validation, and YAML loading for DAP adapter plugins.

mod adapter;
mod error;
mod launch;
mod loader;
mod manifest;
mod target;
mod template;

pub use adapter::{AdapterSpawn, SpawnTransport};
pub use error::PluginError;
pub use launch::{InitSpec, InitStep, InitializeSpec, LaunchSpec, RecordEntry};
pub use loader::{
    PLUGIN_MANIFEST_NAMES, TARGET_MANIFEST_NAMES, default_builtin_dir, load_attachment,
    load_defaults, load_from_dir, load_from_file, load_target_defaults, load_target_from_file,
    load_targets_from_dir, manifest_in_plugin_dir, target_manifest_in_dir, user_plugins_dir,
    user_targets_dir,
};
pub use manifest::PluginManifest;
pub use target::{ResolvedTarget, ResolvedTargetSpawn, TargetConnect, TargetManifest, TargetSpawn};
pub use template::{TemplateContext, resolve_value};
