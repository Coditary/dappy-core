use thiserror::Error;

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("plugin not found: {0}")]
    PluginNotFound(String),

    #[error("no route matched context")]
    NoRoute,

    #[error("duplicate plugin id: {0}")]
    DuplicatePlugin(String),

    #[error("{0}")]
    Other(String),
}
