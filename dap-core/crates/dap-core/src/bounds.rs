use serde_json::{Value, json};

pub const DEFAULT_MAX_VARIABLES: usize = 200;
pub const DEFAULT_MAX_VALUE_CHARS: usize = 4_096;

/// Truncate a DAP `variables` response for agent-friendly output.
pub fn truncate_variables_response(
    body: &Value,
    max_variables: usize,
    max_value_chars: usize,
) -> Value {
    let variables = body
        .get("variables")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let total = variables.len();
    let truncated = total > max_variables;
    let limited = variables
        .into_iter()
        .take(max_variables)
        .map(|entry| truncate_variable_entry(&entry, max_value_chars))
        .collect::<Vec<_>>();
    let mut out = body.clone();
    if let Some(obj) = out.as_object_mut() {
        obj.insert("variables".into(), Value::Array(limited));
        obj.insert("variableCount".into(), json!(total));
        obj.insert("truncated".into(), json!(truncated));
    }
    out
}

fn truncate_variable_entry(entry: &Value, max_value_chars: usize) -> Value {
    let mut out = entry.clone();
    if let Some(value) = out.get("value").and_then(Value::as_str) {
        if value.len() > max_value_chars {
            let truncated = format!("{}…", &value[..max_value_chars]);
            out["value"] = Value::String(truncated);
            out["truncated"] = Value::Bool(true);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_variables_and_values() {
        let body = json!({
            "variables": [
                { "name": "a", "value": "short" },
                { "name": "b", "value": "x".repeat(20) },
                { "name": "c", "value": "tail" },
            ]
        });
        let out = truncate_variables_response(&body, 2, 10);
        assert_eq!(out["truncated"], true);
        assert_eq!(out["variableCount"], 3);
        assert_eq!(out["variables"].as_array().unwrap().len(), 2);
        assert_eq!(out["variables"][1]["truncated"], true);
    }
}
