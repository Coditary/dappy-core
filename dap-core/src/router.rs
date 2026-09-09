use dap_plugin_api::PluginManifest;

use crate::gateway::resolve_route;
use crate::registry::PluginRegistry;
use crate::session::LaunchOptions;

/// Result of routing a launch request to an adapter plugin.
#[derive(Debug, Clone)]
pub struct RouteMatch {
    pub adapter_id: String,
    pub manifest: PluginManifest,
}

/// Routes launch requests via `protocol-gateway`.
#[derive(Debug, Default)]
pub struct Router;

impl Router {
    pub fn new() -> Self {
        Self
    }

    pub fn resolve(&self, registry: &PluginRegistry, launch: &LaunchOptions) -> Option<RouteMatch> {
        resolve_route(registry, launch).ok()
    }
}
