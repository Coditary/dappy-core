//! Generic multi-instance registry and lifecycle management.
//!
//! This crate is intentionally protocol-agnostic so it can back DAP sessions,
//! LSP servers, or any other long-lived child process / service instance.

mod error;
mod handle;
mod id;
mod manager;
mod spec;
mod state;
mod store;

pub use error::InstanceError;
pub use handle::InstanceHandle;
pub use id::InstanceId;
pub use manager::{InstanceManager, session_record_from_handle};
pub use spec::{ControlPort, InstanceSpec};
pub use state::InstanceState;
pub use store::{SessionRecord, SessionStore, is_control_port_in_use, is_pid_alive};
