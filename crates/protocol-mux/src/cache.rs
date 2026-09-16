use serde_json::{Value, json};

use crate::breakpoint_state::BreakpointTracker;
use crate::message::{MessageKind, command, event_name, message_kind};

/// Cached messages replayed to clients that attach after session init.
#[derive(Debug, Default, Clone)]
pub struct LateJoinCache {
    initialized: Option<Value>,
    last_stopped: Option<Value>,
    initialize_body: Option<Value>,
    breakpoints: BreakpointTracker,
}

impl LateJoinCache {
    pub fn observe(&mut self, message: &Value) {
        if message_kind(message) != MessageKind::Event {
            return;
        }
        match event_name(message) {
            Some("initialized") => self.initialized = Some(message.clone()),
            Some("stopped") => self.last_stopped = Some(message.clone()),
            Some("breakpoint") => self.breakpoints.observe_breakpoint_event(message),
            _ => {}
        }
    }

    pub fn observe_response(&mut self, message: &Value) {
        if message_kind(message) != MessageKind::Response {
            return;
        }
        match message.get("command").and_then(Value::as_str) {
            Some("initialize") if message.get("success").and_then(Value::as_bool) == Some(true) => {
                self.initialize_body = message.get("body").cloned();
            }
            _ => {}
        }
        self.breakpoints.observe_set_breakpoints_response(message);
    }

    pub fn observe_client_request(&mut self, backend_seq: i64, message: &Value) {
        match command(message) {
            Some("setBreakpoints") => self
                .breakpoints
                .track_set_breakpoints_request(backend_seq, message),
            Some("setExceptionBreakpoints") => {
                self.breakpoints
                    .track_set_exception_breakpoints_request(message);
            }
            _ => {}
        }
    }

    pub fn breakpoint_snapshot(&self) -> Value {
        self.breakpoints.snapshot()
    }

    pub fn replay(&self) -> Vec<Value> {
        let mut out = Vec::with_capacity(3);
        if let Some(body) = &self.initialize_body {
            out.push(json!({
                "type": "response",
                "seq": 0,
                "request_seq": 0,
                "success": true,
                "command": "initialize",
                "body": body,
            }));
        }
        if let Some(msg) = &self.initialized {
            out.push(msg.clone());
        }
        if let Some(msg) = &self.last_stopped {
            out.push(msg.clone());
        }
        out
    }

    pub fn initialize_body(&self) -> Option<&Value> {
        self.initialize_body.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_initialize_response_for_late_join() {
        let mut cache = LateJoinCache::default();
        cache.observe_response(&json!({
            "type": "response",
            "command": "initialize",
            "success": true,
            "body": { "supportsStepBack": false }
        }));
        cache.observe(&json!({ "type": "event", "event": "initialized" }));

        let replay = cache.replay();
        assert_eq!(replay.len(), 2);
        assert_eq!(replay[0]["request_seq"], 0);
        assert_eq!(replay[0]["command"], "initialize");
        assert_eq!(replay[1]["event"], "initialized");
    }
}
