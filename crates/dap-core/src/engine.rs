use std::sync::OnceLock;

use instance_manager::{InstanceManager, InstanceSpec, InstanceState};

use crate::registry::PluginRegistry;
use crate::router::Router;
use crate::session::{LaunchOptions, SessionHandle};
use crate::target_registry::TargetRegistry;

/// Top-level orchestrator: registry + routing + instance tracking.
#[derive(Debug)]
pub struct DapEngine {
    pub instances: InstanceManager,
    pub registry: PluginRegistry,
    pub target_registry: TargetRegistry,
    pub router: Router,
}

impl DapEngine {
    pub fn new() -> Self {
        Self {
            instances: InstanceManager::new(),
            registry: PluginRegistry::new(),
            target_registry: TargetRegistry::new(),
            router: Router::new(),
        }
    }

    /// Construct engine and load builtin plugin manifests from `dap-plugins/builtin`.
    pub fn with_builtin_plugins() -> anyhow::Result<Self> {
        let mut engine = Self::new();
        engine.load_builtin_plugins()?;
        Ok(engine)
    }

    /// Headless REPL session engine: cached plugin registry, fresh instance tracking.
    pub fn for_headless_session() -> anyhow::Result<Self> {
        Ok(Self {
            instances: InstanceManager::new(),
            registry: cached_builtin_registry()?.clone(),
            target_registry: cached_builtin_target_registry()?.clone(),
            router: Router::new(),
        })
    }

    pub fn load_builtin_plugins(&mut self) -> anyhow::Result<()> {
        self.registry.load_builtin()?;
        self.registry = patch_registry_from_env(self.registry.clone());
        self.target_registry.load_builtin()?;
        Ok(())
    }

    pub fn load_plugins_from_dir(&mut self, dir: &std::path::Path) -> anyhow::Result<()> {
        self.registry.load_from_dir(dir)?;
        self.registry = patch_registry_from_env(self.registry.clone());
        Ok(())
    }

    pub fn adapter_spawn(&self, adapter_id: &str) -> Option<dap_plugin_api::AdapterSpawn> {
        self.registry
            .get(adapter_id)
            .map(|manifest| manifest.adapter.clone())
    }

    /// Resolve adapter spawn spec for a launch request (routing via protocol-gateway).
    pub fn resolve_adapter_spawn(
        &self,
        launch: &LaunchOptions,
    ) -> anyhow::Result<dap_plugin_api::AdapterSpawn> {
        let route = self
            .router
            .resolve(&self.registry, launch)
            .ok_or_else(|| anyhow::anyhow!("no adapter matched launch request"))?;
        Ok(route.manifest.adapter)
    }

    pub fn with_router(router: Router) -> Self {
        Self {
            instances: InstanceManager::new(),
            registry: PluginRegistry::new(),
            target_registry: TargetRegistry::new(),
            router,
        }
    }

    pub fn register_plugin(&mut self, manifest: dap_plugin_api::PluginManifest) {
        let manifest = patch_adapter_from_env(manifest);
        self.registry.register(manifest);
    }

    /// Resolve adapter and register a session instance (no backend spawn yet).
    pub async fn prepare_session(&self, launch: LaunchOptions) -> anyhow::Result<SessionHandle> {
        let route = self
            .router
            .resolve(&self.registry, &launch)
            .ok_or_else(|| anyhow::anyhow!("no adapter matched launch request"))?;

        let label = launch.program.clone();
        let mut spec =
            InstanceSpec::new("dap-session").with_tag("adapter", route.adapter_id.clone());
        if let Some(label) = label {
            spec = spec.with_label(label);
        }

        let instance_id = self.instances.register(spec).await?;
        self.instances
            .set_state(&instance_id, InstanceState::Starting)
            .await?;

        Ok(SessionHandle {
            instance_id,
            adapter_id: route.adapter_id,
            control_port: None,
        })
    }

    pub async fn set_control_port(
        &self,
        session: &SessionHandle,
        port: u16,
    ) -> anyhow::Result<SessionHandle> {
        self.instances
            .set_control_port(&session.instance_id, port)
            .await?;
        Ok(SessionHandle {
            instance_id: session.instance_id.clone(),
            adapter_id: session.adapter_id.clone(),
            control_port: Some(port),
        })
    }

    pub async fn mark_running(&self, session: &SessionHandle) -> anyhow::Result<()> {
        self.instances
            .set_pid(&session.instance_id, std::process::id())
            .await?;
        self.instances
            .set_state(&session.instance_id, InstanceState::Running)
            .await?;
        Ok(())
    }
}

impl Default for DapEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn cached_builtin_target_registry() -> anyhow::Result<&'static TargetRegistry> {
    static REGISTRY: OnceLock<TargetRegistry> = OnceLock::new();
    if let Some(registry) = REGISTRY.get() {
        return Ok(registry);
    }
    let mut registry = TargetRegistry::new();
    registry.load_builtin()?;
    Ok(REGISTRY.get_or_init(|| registry))
}

fn cached_builtin_registry() -> anyhow::Result<&'static PluginRegistry> {
    static REGISTRY: OnceLock<PluginRegistry> = OnceLock::new();
    if let Some(registry) = REGISTRY.get() {
        return Ok(registry);
    }
    let mut registry = PluginRegistry::new();
    registry.load_builtin()?;
    registry = patch_registry_from_env(registry);
    Ok(REGISTRY.get_or_init(|| registry))
}

fn patch_registry_from_env(registry: PluginRegistry) -> PluginRegistry {
    let mut patched = PluginRegistry::new();
    for manifest in registry.list() {
        patched.register(patch_adapter_from_env(manifest.clone()));
    }
    patched
}

fn patch_adapter_from_env(
    mut manifest: dap_plugin_api::PluginManifest,
) -> dap_plugin_api::PluginManifest {
    if manifest.id == "rust" {
        if let Ok(command) = std::env::var("LLDB_DAP") {
            manifest.adapter.command = command;
        }
    }
    if manifest.id == "python" {
        if let Ok(command) = std::env::var("PYTHON") {
            manifest.adapter.command = command;
        }
    }
    manifest
}
