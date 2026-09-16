use anyhow::{Context, Result};
use dap_protocol::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Adapter features reported in the `initialize` response body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterCapabilities {
    #[serde(default)]
    pub supports_configuration_done_request: bool,
    #[serde(default)]
    pub supports_function_breakpoints: bool,
    #[serde(default)]
    pub supports_conditional_breakpoints: bool,
    #[serde(default)]
    pub supports_hit_conditional_breakpoints: bool,
    #[serde(default)]
    pub supports_evaluate_for_hovers: bool,
    #[serde(default)]
    pub supports_step_back: bool,
    #[serde(default)]
    pub supports_set_variable: bool,
    #[serde(default)]
    pub supports_set_expression: bool,
    #[serde(default)]
    pub supports_goto_targets_request: bool,
    #[serde(default)]
    pub supports_goto_request: bool,
    #[serde(default)]
    pub supports_completions_request: bool,
    #[serde(default)]
    pub supports_breakpoint_locations_request: bool,
    #[serde(default)]
    pub supports_data_breakpoints: bool,
    #[serde(default)]
    pub supports_disassemble_request: bool,
    #[serde(default)]
    pub supports_instruction_breakpoints: bool,
    #[serde(default)]
    pub supports_exception_filter_options: bool,
    #[serde(default)]
    pub supports_log_points: bool,
    #[serde(default)]
    pub supports_restart_frame: bool,
    #[serde(default)]
    pub supports_read_memory_request: bool,
    #[serde(default)]
    pub supports_write_memory_request: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exception_breakpoint_filters: Option<Vec<Value>>,
    /// Full initialize body for forward compatibility.
    #[serde(skip)]
    pub raw: Option<Value>,
}

impl AdapterCapabilities {
    /// Conservative defaults when capabilities are unknown (e.g. late attach).
    pub fn unknown() -> Self {
        Self::default()
    }

    pub fn from_initialize_message(message: &Message) -> Result<Self> {
        match message {
            Message::Response(response) => {
                let body = response
                    .body
                    .as_ref()
                    .context("initialize response missing body")?;
                Self::from_json(body)
            }
            other => anyhow::bail!("expected initialize response, got {:?}", other),
        }
    }

    pub fn from_json(value: &Value) -> Result<Self> {
        let mut caps: Self =
            serde_json::from_value(value.clone()).context("parse adapter capabilities")?;
        caps.raw = Some(value.clone());
        Ok(caps)
    }

    pub fn supports_conditional_breakpoints(&self) -> bool {
        self.supports_conditional_breakpoints
    }
}

/// Interpret a DAP `evaluate` result as a boolean condition.
pub fn condition_result_is_true(body: &Value) -> bool {
    let result = body
        .get("result")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");

    if result.is_empty() {
        return false;
    }

    let lower = result.to_ascii_lowercase();
    match lower.as_str() {
        "true" | "1" | "yes" => return true,
        "false" | "0" | "no" | "none" | "nil" | "null" => return false,
        _ => {}
    }

    if let Ok(number) = result.parse::<f64>() {
        return number != 0.0;
    }

    // lldb-style: "(bool) true", "true\n", etc.
    lower.contains("true") && !lower.contains("false")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_initialize_capabilities() {
        let body = serde_json::json!({
            "supportsConfigurationDoneRequest": true,
            "supportsConditionalBreakpoints": true,
            "supportsStepBack": false,
        });
        let caps = AdapterCapabilities::from_json(&body).unwrap();
        assert!(caps.supports_configuration_done_request);
        assert!(caps.supports_conditional_breakpoints);
        assert!(!caps.supports_step_back);
    }

    #[test]
    fn condition_truthiness() {
        assert!(condition_result_is_true(
            &serde_json::json!({ "result": "true" })
        ));
        assert!(condition_result_is_true(
            &serde_json::json!({ "result": "1" })
        ));
        assert!(!condition_result_is_true(
            &serde_json::json!({ "result": "false" })
        ));
        assert!(!condition_result_is_true(
            &serde_json::json!({ "result": "0" })
        ));
        assert!(!condition_result_is_true(
            &serde_json::json!({ "result": "" })
        ));
    }

    #[test]
    fn from_initialize_message_rejects_non_response() {
        let message = dap_protocol::Message::Event(dap_protocol::Event {
            seq: 1,
            event: "initialized".into(),
            body: None,
        });
        assert!(AdapterCapabilities::from_initialize_message(&message).is_err());
    }

    #[test]
    fn from_initialize_message_parses_response_body() {
        let message = dap_protocol::Message::Response(dap_protocol::Response {
            seq: 1,
            request_seq: 1,
            success: true,
            command: Some("initialize".into()),
            message: None,
            body: Some(serde_json::json!({
                "supportsSetVariable": true,
                "supportsExceptionFilterOptions": true,
            })),
        });
        let caps = AdapterCapabilities::from_initialize_message(&message).unwrap();
        assert!(caps.supports_set_variable);
        assert!(caps.supports_exception_filter_options);
    }
}
