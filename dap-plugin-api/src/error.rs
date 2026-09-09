use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),

    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },

    #[error("failed to parse {path}: {source}")]
    Parse {
        path: String,
        source: serde_yaml::Error,
    },
}
