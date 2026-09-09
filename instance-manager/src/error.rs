use thiserror::Error;

#[derive(Debug, Error)]
pub enum InstanceError {
    #[error("instance not found: {0}")]
    NotFound(String),

    #[error("instance already exists: {0}")]
    AlreadyExists(String),

    #[error("invalid instance state transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },

    #[error("{0}")]
    Other(String),
}
