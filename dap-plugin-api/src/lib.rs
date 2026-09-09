//! Plugin manifest types, validation, and YAML loading for DAP adapter plugins.

mod adapter;
mod error;
mod loader;
mod manifest;

pub use adapter::{AdapterSpawn, SpawnTransport};
pub use error::PluginError;
pub use loader::{
    default_builtin_dir, load_defaults, load_from_dir, load_from_file, user_plugins_dir,
};
pub use manifest::PluginManifest;
