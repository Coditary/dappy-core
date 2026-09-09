use thiserror::Error;

#[derive(Debug, Error)]
pub enum MuxError {
    #[error("client not found")]
    ClientNotFound,

    #[error("upstream not connected")]
    UpstreamNotConnected,

    #[error("{0}")]
    Other(String),
}
