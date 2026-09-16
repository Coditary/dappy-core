//! Generic multiplexer: N clients ↔ 1 upstream with sequence remapping.

mod breakpoint_state;
mod cache;
mod cancel;
mod client;
mod error;
mod message;
mod remapper;
mod session;

pub use cache::LateJoinCache;
pub use cancel::translate_cancel;
pub use client::{ClientId, ClientRole, Id};
pub use error::MuxError;
pub use message::{
    MessageKind, command, event, event_name, message_kind, request, request_seq, response, seq,
    set_request_seq, set_seq,
};
pub use remapper::{BackendSeq, ClientSeq, MessageRemapper};
pub use session::{BREAKPOINT_SNAPSHOT_COMMAND, ClientEndpoint, MultiplexSession, Multiplexer};
