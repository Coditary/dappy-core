use serde_json::Value;

use crate::message::{cancel_request_id, set_cancel_request_id};
use crate::remapper::{ClientSeq, MessageRemapper};

/// Rewrites `cancel.arguments.requestId` from a client sequence to the upstream
/// sequence when a mapping exists.
///
/// Only call on the editor path before recording the cancel request's own mapping.
pub fn translate_cancel(request: &mut Value, remapper: &MessageRemapper) -> bool {
    let Some(referenced) = cancel_request_id(request) else {
        return false;
    };
    let referenced = ClientSeq::new(referenced);
    match remapper.lookup_backend(referenced) {
        Some(backend) => set_cancel_request_id(request, backend.raw()),
        None => false,
    }
}
