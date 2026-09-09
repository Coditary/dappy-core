use async_trait::async_trait;

use crate::route::RouteContext;
use crate::spawn::SpawnSpec;

/// A registrable backend plugin (DAP adapter, LSP server, …).
#[async_trait]
pub trait GatewayPlugin: Send + Sync {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;

    /// Return true when this plugin should handle the given routing context.
    fn matches(&self, ctx: &RouteContext) -> bool;

    /// How to spawn or connect to the upstream implementation.
    fn spawn_spec(&self) -> SpawnSpec;
}

/// Static plugin definition for tests and YAML-loaded manifests.
#[derive(Debug, Clone)]
pub struct StaticPlugin {
    pub id: String,
    pub name: String,
    pub launch_types: Vec<String>,
    pub file_extensions: Vec<String>,
    pub spawn: SpawnSpec,
}

#[async_trait]
impl GatewayPlugin for StaticPlugin {
    fn id(&self) -> &str {
        &self.id
    }

    fn display_name(&self) -> &str {
        &self.name
    }

    fn matches(&self, ctx: &RouteContext) -> bool {
        if let Some(explicit) = &ctx.explicit_plugin {
            return explicit == &self.id;
        }
        if let Some(lt) = &ctx.launch_type {
            if self.launch_types.iter().any(|t| t == lt) {
                return true;
            }
        }
        if let Some(path) = &ctx.program_path {
            if let Some(ext) = path.rsplit('.').next() {
                if self.file_extensions.iter().any(|e| e == ext) {
                    return true;
                }
            }
        }
        false
    }

    fn spawn_spec(&self) -> SpawnSpec {
        self.spawn.clone()
    }
}
