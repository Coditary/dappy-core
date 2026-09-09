use std::sync::Arc;

use crate::error::GatewayError;
use crate::plugin::GatewayPlugin;
use crate::route::{RouteContext, RouteMatch};

/// In-memory plugin registry.
#[derive(Default)]
pub struct PluginRegistry {
    plugins: Vec<Arc<dyn GatewayPlugin>>,
    default_plugin: Option<String>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, plugin: Arc<dyn GatewayPlugin>) -> Result<(), GatewayError> {
        if self.plugins.iter().any(|p| p.id() == plugin.id()) {
            return Err(GatewayError::DuplicatePlugin(plugin.id().to_string()));
        }
        self.plugins.push(plugin);
        Ok(())
    }

    pub fn set_default(&mut self, plugin_id: impl Into<String>) {
        self.default_plugin = Some(plugin_id.into());
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn GatewayPlugin>> {
        self.plugins.iter().find(|p| p.id() == id).cloned()
    }

    pub fn list_ids(&self) -> Vec<String> {
        self.plugins.iter().map(|p| p.id().to_string()).collect()
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

/// Priority-based router over a plugin registry.
pub struct Router<'a> {
    registry: &'a PluginRegistry,
}

impl<'a> Router<'a> {
    pub fn new(registry: &'a PluginRegistry) -> Self {
        Self { registry }
    }

    pub fn resolve(&self, ctx: &RouteContext) -> Result<RouteMatch, GatewayError> {
        if let Some(id) = &ctx.explicit_plugin {
            if self.registry.get(id).is_some() {
                return Ok(RouteMatch {
                    plugin_id: id.clone(),
                });
            }
            return Err(GatewayError::PluginNotFound(id.clone()));
        }

        for plugin in &self.registry.plugins {
            if plugin.matches(ctx) {
                return Ok(RouteMatch {
                    plugin_id: plugin.id().to_string(),
                });
            }
        }

        if let Some(default_id) = &self.registry.default_plugin {
            if self.registry.get(default_id).is_some() {
                return Ok(RouteMatch {
                    plugin_id: default_id.clone(),
                });
            }
        }

        Err(GatewayError::NoRoute)
    }
}
