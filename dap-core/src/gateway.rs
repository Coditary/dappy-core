use std::sync::Arc;

use dap_plugin_api::{AdapterSpawn, PluginManifest, SpawnTransport};
use protocol_gateway::{
    GatewayError, PluginRegistry as GatewayRegistry, RouteContext, Router as GatewayRouter,
    SpawnSpec, StaticPlugin,
};

use crate::registry::PluginRegistry;
use crate::router::RouteMatch;
use crate::rust_routing::looks_like_cargo_binary;
use crate::session::LaunchOptions;

pub fn manifest_to_static_plugin(manifest: &PluginManifest) -> StaticPlugin {
    StaticPlugin {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        launch_types: manifest.launch_types.clone(),
        file_extensions: manifest.file_extensions.clone(),
        spawn: adapter_to_spawn_spec(&manifest.adapter),
    }
}

pub fn build_gateway_registry(registry: &PluginRegistry) -> GatewayRegistry {
    let mut gw = GatewayRegistry::new();
    for manifest in registry.list() {
        let _ = gw.register(Arc::new(manifest_to_static_plugin(manifest)));
    }
    if gw.get("fake").is_some() {
        gw.set_default("fake");
    }
    gw
}

pub fn launch_to_route_context(launch: &LaunchOptions) -> RouteContext {
    let mut ctx = RouteContext::new();
    if let Some(adapter) = &launch.adapter {
        ctx = ctx.with_explicit_plugin(adapter.clone());
    }
    if let Some(program) = &launch.program {
        ctx = ctx.with_program_path(program.clone());
        if launch.adapter.is_none()
            && launch.extra.get("type").and_then(|v| v.as_str()).is_none()
            && looks_like_cargo_binary(program)
        {
            ctx = ctx.with_launch_type("rust");
        }
    }
    if let Some(launch_type) = launch.extra.get("type").and_then(|v| v.as_str()) {
        ctx = ctx.with_launch_type(launch_type);
    }
    ctx
}

pub fn resolve_route(
    registry: &PluginRegistry,
    launch: &LaunchOptions,
) -> Result<RouteMatch, GatewayError> {
    let gw = build_gateway_registry(registry);
    let router = GatewayRouter::new(&gw);
    let ctx = launch_to_route_context(launch);
    let matched = router.resolve(&ctx)?;
    let manifest = registry
        .get(&matched.plugin_id)
        .ok_or_else(|| GatewayError::PluginNotFound(matched.plugin_id.clone()))?;
    Ok(RouteMatch {
        adapter_id: matched.plugin_id,
        manifest: manifest.clone(),
    })
}

pub fn spawn_spec_to_adapter(spec: &SpawnSpec) -> AdapterSpawn {
    match spec {
        SpawnSpec::Stdio { command, args } => AdapterSpawn {
            transport: SpawnTransport::Stdio,
            command: command.clone(),
            args: args.clone(),
        },
        SpawnSpec::Tcp { host, port } => AdapterSpawn {
            transport: SpawnTransport::Tcp,
            command: format!("{host}:{port}"),
            args: vec![],
        },
    }
}

fn adapter_to_spawn_spec(adapter: &AdapterSpawn) -> SpawnSpec {
    match adapter.transport {
        SpawnTransport::Stdio => SpawnSpec::stdio(adapter.command.clone(), adapter.args.clone()),
        SpawnTransport::Tcp => {
            let (host, port) = adapter
                .command
                .split_once(':')
                .and_then(|(h, p)| p.parse().ok().map(|port| (h.to_string(), port)))
                .unwrap_or_else(|| (adapter.command.clone(), 0));
            SpawnSpec::Tcp { host, port }
        }
    }
}
