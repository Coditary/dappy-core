//! Generic gateway: registry, routing, upstream spawn specifications.

mod error;
mod plugin;
mod registry;
mod route;
mod spawn;

pub use error::GatewayError;
pub use plugin::{GatewayPlugin, StaticPlugin};
pub use registry::{PluginRegistry, Router};
pub use route::{RouteContext, RouteMatch};
pub use spawn::{SpawnSpec, SpawnTransport};
