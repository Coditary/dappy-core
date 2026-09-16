use std::path::Path;

use anyhow::Context;
use dap_plugin_api::{PluginManifest, default_builtin_dir, load_defaults, load_from_dir};

/// Registry of adapter plugin manifests with disk-backed loading.
#[derive(Debug, Default, Clone)]
pub struct PluginRegistry {
    plugins: Vec<PluginManifest>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, manifest: PluginManifest) {
        self.plugins.retain(|p| p.id != manifest.id);
        self.plugins.push(manifest);
    }

    pub fn get(&self, id: &str) -> Option<&PluginManifest> {
        self.plugins.iter().find(|p| p.id == id)
    }

    pub fn find_by_launch_type(&self, launch_type: &str) -> Option<&PluginManifest> {
        self.plugins
            .iter()
            .find(|p| p.launch_types.iter().any(|t| t == launch_type))
    }

    pub fn find_by_extension(&self, extension: &str) -> Option<&PluginManifest> {
        let ext = extension.trim_start_matches('.');
        self.plugins
            .iter()
            .find(|p| p.file_extensions.iter().any(|candidate| candidate == ext))
    }

    pub fn list(&self) -> &[PluginManifest] {
        &self.plugins
    }

    /// Load manifests from a plugin root (`<id>/plugin.yaml` or flat `*.yaml`).
    pub fn load_from_dir(&mut self, dir: &Path) -> anyhow::Result<()> {
        for manifest in load_from_dir(dir).context("load plugin manifests")? {
            self.register(manifest);
        }
        Ok(())
    }

    /// Load builtin plugins and optional user overrides from config dirs.
    pub fn load_defaults(&mut self) -> anyhow::Result<()> {
        for manifest in load_defaults().context("load default plugin manifests")? {
            self.register(manifest);
        }
        Ok(())
    }

    /// Load builtin plugins from `dap-plugins/builtin` (or `$DAP_PLUGINS_DIR`).
    pub fn load_builtin(&mut self) -> anyhow::Result<()> {
        self.load_from_dir(&default_builtin_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dap_plugin_api::{AdapterSpawn, SpawnTransport};

    fn sample_manifest(id: &str) -> PluginManifest {
        PluginManifest {
            id: id.into(),
            name: id.into(),
            version: "0.1.0".into(),
            languages: vec![],
            launch_types: vec![id.into()],
            file_extensions: vec![],
            adapter: AdapterSpawn {
                transport: SpawnTransport::Stdio,
                command: id.into(),
                args: vec![],
            },
            initialize: None,
            launch: None,
            init: None,
            attachment: None,
            source_path: None,
        }
    }

    #[test]
    fn register_and_replace_plugin() {
        let mut registry = PluginRegistry::new();
        registry.register(sample_manifest("fake"));
        registry.register(sample_manifest("fake"));
        assert_eq!(registry.list().len(), 1);
        assert_eq!(registry.get("fake").unwrap().adapter.command, "fake");
    }

    #[test]
    fn find_by_launch_type_and_extension() {
        let mut registry = PluginRegistry::new();
        let mut manifest = sample_manifest("python");
        manifest.file_extensions = vec!["py".into()];
        registry.register(manifest);
        assert!(registry.find_by_launch_type("python").is_some());
        assert!(registry.find_by_extension("py").is_some());
        assert!(registry.find_by_launch_type("missing").is_none());
    }

    #[test]
    fn load_builtin_plugins() {
        let mut registry = PluginRegistry::new();
        registry.load_builtin().expect("load builtin plugins");
        assert!(registry.get("fake").is_some());
        assert!(registry.get("rust").is_some());
    }
}

#[cfg(test)]
mod target_registry_tests {
    use super::*;
    use crate::target_registry::TargetRegistry;

    #[test]
    fn load_builtin_targets() {
        let mut registry = TargetRegistry::new();
        registry.load_builtin().expect("load builtin targets");
        assert!(registry.get("gdbserver").is_some());
        assert!(registry.get("qemu-x86-kernel").is_some());
    }
}
