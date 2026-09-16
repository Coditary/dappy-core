use serde_json::{Value, json};

/// DAP-style message kind inferred from a JSON envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Request,
    Response,
    Event,
    Unknown,
}

pub fn message_kind(value: &Value) -> MessageKind {
    match value.get("type").and_then(Value::as_str) {
        Some("request") => MessageKind::Request,
        Some("response") => MessageKind::Response,
        Some("event") => MessageKind::Event,
        _ => MessageKind::Unknown,
    }
}

pub fn seq(value: &Value) -> Option<i64> {
    value.get("seq").and_then(Value::as_i64)
}

pub fn set_seq(value: &mut Value, seq: i64) {
    if let Some(obj) = value.as_object_mut() {
        obj.insert("seq".to_string(), json!(seq));
    }
}

pub fn request_seq(value: &Value) -> Option<i64> {
    value
        .get("requestSeq")
        .or_else(|| value.get("request_seq"))
        .and_then(Value::as_i64)
}

pub fn set_request_seq(value: &mut Value, request_seq: i64) {
    if let Some(obj) = value.as_object_mut() {
        obj.remove("requestSeq");
        obj.insert("request_seq".to_string(), json!(request_seq));
    }
}

pub fn command(value: &Value) -> Option<&str> {
    value.get("command").and_then(Value::as_str)
}

pub fn event_name(value: &Value) -> Option<&str> {
    value.get("event").and_then(Value::as_str)
}

pub fn cancel_request_id(value: &mut Value) -> Option<i64> {
    if command(value) != Some("cancel") {
        return None;
    }
    value
        .get_mut("arguments")
        .and_then(Value::as_object_mut)
        .and_then(|args| args.get_mut("requestId"))
        .and_then(|id| {
            let current = id.as_i64()?;
            *id = json!(current);
            id.as_i64()
        })
}

pub fn set_cancel_request_id(value: &mut Value, request_id: i64) -> bool {
    if command(value) != Some("cancel") {
        return false;
    }
    let Some(args) = value.get_mut("arguments").and_then(Value::as_object_mut) else {
        return false;
    };
    args.insert("requestId".to_string(), json!(request_id));
    true
}

pub fn request(seq_no: i64, command: &str, arguments: Option<Value>) -> Value {
    json!({
        "type": "request",
        "seq": seq_no,
        "command": command,
        "arguments": arguments.unwrap_or(json!({})),
    })
}

pub fn response(seq: i64, request_seq: i64, success: bool, body: Option<Value>) -> Value {
    json!({
        "type": "response",
        "seq": seq,
        "request_seq": request_seq,
        "success": success,
        "body": body,
    })
}

pub fn event(seq: i64, name: &str, body: Option<Value>) -> Value {
    json!({
        "type": "event",
        "seq": seq,
        "event": name,
        "body": body.unwrap_or(json!({})),
    })
}
