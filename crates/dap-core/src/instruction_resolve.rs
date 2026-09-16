use serde_json::Value;

/// Extract a source location from a `disassemble` response for emulated instruction breakpoints.
pub fn resolve_instruction_location(body: &Value) -> Option<(String, i64)> {
    let instruction = body
        .get("instructions")
        .and_then(Value::as_array)
        .and_then(|items| items.first())?;
    let line = instruction.get("line").and_then(Value::as_i64)?;
    let path = instruction
        .get("location")
        .and_then(|location| location.get("path"))
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())?;
    Some((path.to_string(), line))
}

/// Stable key for tracking emulated instruction breakpoints.
pub fn instruction_breakpoint_key(memory_reference: &str, offset: i64) -> String {
    format!("{memory_reference}:{offset}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_instruction_location_from_disassemble_body() {
        let body = json!({
            "instructions": [{
                "line": 42,
                "location": { "path": "/fake/main.py" }
            }]
        });
        assert_eq!(
            resolve_instruction_location(&body),
            Some(("/fake/main.py".into(), 42))
        );
    }
}
