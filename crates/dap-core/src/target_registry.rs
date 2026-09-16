use dap_plugin_api::{TargetManifest, load_target_defaults, load_targets_from_dir};

/// Registry of RSP target manifests (`target.yaml`), parallel to [`PluginRegistry`].
#[derive(Debug, Clone, Default)]
pub struct TargetRegistry {
    targets: Vec<TargetManifest>,
}

impl TargetRegistry {
    pub fn new() -> Self {
        Self {
            targets: Vec::new(),
        }
    }

    pub fn register(&mut self, manifest: TargetManifest) {
        if let Some(existing) = self.targets.iter().position(|entry| entry.id == manifest.id) {
            self.targets[existing] = manifest;
        } else {
            self.targets.push(manifest);
        }
    }

    pub fn get(&self, id: &str) -> Option<&TargetManifest> {
        self.targets.iter().find(|manifest| manifest.id == id)
    }

    pub fn list(&self) -> &[TargetManifest] {
        &self.targets
    }

    pub fn load_builtin(&mut self) -> anyhow::Result<()> {
        for manifest in load_target_defaults()? {
            self.register(manifest);
        }
        Ok(())
    }

    pub fn load_from_dir(&mut self, dir: &std::path::Path) -> anyhow::Result<()> {
        for manifest in load_targets_from_dir(dir)? {
            self.register(manifest);
        }
        Ok(())
    }
}
