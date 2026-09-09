use std::time::Duration;

use anyhow::{Context, Result};
use dap_protocol::Message;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::capabilities::AdapterCapabilities;
use crate::control_client::ControlClient;
use crate::python_routing::{python_launch_arguments, wants_python_launch_hints};
use crate::rust_routing::wants_rust_launch_hints;

/// Configuration for a headless DAP initialization sequence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInitConfig {
    pub program: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_id: Option<String>,
    #[serde(default)]
    pub client_id: String,
}

impl SessionInitConfig {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            adapter_id: None,
            client_id: "dap-core".into(),
        }
    }

    pub fn with_adapter_id(mut self, adapter_id: impl Into<String>) -> Self {
        self.adapter_id = Some(adapter_id.into());
        self
    }
}

/// Known entry location after init (avoids an extra stackTrace during REPL connect).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntryLocation {
    pub path: String,
    pub line: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<i64>,
}

/// Result of a completed headless initialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInitResult {
    pub ready: bool,
    pub stop_reason: Option<String>,
    pub thread_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_location: Option<SessionEntryLocation>,
    pub capabilities: AdapterCapabilities,
}

/// Run initialize → launch → initialized → configurationDone → stopped over a control client.
pub async fn run_session_init(
    client: &mut ControlClient,
    config: &SessionInitConfig,
) -> Result<SessionInitResult> {
    run_debug_request(client, config, "launch", &build_launch_arguments(config)).await
}

/// Run initialize → attach/launch → initialized → configurationDone → stopped.
pub async fn run_debug_request(
    client: &mut ControlClient,
    config: &SessionInitConfig,
    request: &str,
    arguments: &serde_json::Value,
) -> Result<SessionInitResult> {
    let init_args = build_initialize_arguments(config);

    let init_message = client
        .dap_request_preserve_events("initialize", Some(init_args))
        .await
        .context("initialize")?;
    let capabilities = AdapterCapabilities::from_initialize_message(&init_message)?;
    client.set_capabilities(capabilities.clone());

    let mut entry_location = None;

    if wants_python_launch_hints(config.adapter_id.as_deref()) {
        let launch_seq = client
            .send_dap_request_and_wait_for_event(
                request,
                Some(arguments.clone()),
                "initialized",
                Duration::from_secs(30),
            )
            .await
            .context("wait for initialized event after launch")?;

        let program_path = arguments
            .get("program")
            .and_then(|value| value.as_str())
            .unwrap_or(&config.program);
        client
            .dap_request_preserve_events(
                "setBreakpoints",
                Some(json!({
                    "source": { "path": program_path },
                    "breakpoints": [{ "line": 1 }],
                })),
            )
            .await
            .context("set entry breakpoint")?;
        entry_location = Some(SessionEntryLocation {
            path: program_path.to_string(),
            line: 1,
            frame_id: Some(1),
        });

        client
            .dap_request_preserve_events("configurationDone", Some(json!({})))
            .await
            .context("configurationDone")?;

        client
            .wait_for_response_seq(launch_seq)
            .await
            .with_context(|| format!("wait for {request} response"))?;
    } else {
        client
            .dap_request_preserve_events(request, Some(arguments.clone()))
            .await
            .with_context(|| request.to_string())?;

        read_until_event(client, "initialized", Duration::from_secs(30))
            .await
            .context("wait for initialized event")?;

        client
            .dap_request_preserve_events("configurationDone", Some(json!({})))
            .await
            .context("configurationDone")?;
    }

    let stopped = read_until_event(client, "stopped", Duration::from_secs(30))
        .await
        .context("wait for stopped event")?;

    let (stop_reason, thread_id) = match &stopped {
        Message::Event(event) => (
            event
                .body
                .as_ref()
                .and_then(|b| b.get("reason"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            event
                .body
                .as_ref()
                .and_then(|b| b.get("threadId"))
                .and_then(|v| v.as_i64()),
        ),
        _ => (None, None),
    };

    Ok(SessionInitResult {
        ready: true,
        stop_reason,
        thread_id,
        entry_location,
        capabilities,
    })
}

/// Build DAP `initialize` arguments for a frontend or headless client.
pub fn build_initialize_arguments(config: &SessionInitConfig) -> serde_json::Value {
    json!({
        "clientID": config.client_id,
        "adapterID": initialize_adapter_id(config),
        "linesStartAt1": true,
        "columnsStartAt1": true,
        "pathFormat": "path",
        // dap-proxy answers runInTerminal on behalf of headless clients.
        "supportsRunInTerminalRequest": true,
    })
}

fn initialize_adapter_id(config: &SessionInitConfig) -> String {
    match config.adapter_id.as_deref() {
        Some("python") => "debugpy".into(),
        Some(other) => other.into(),
        None => "dap-proxy".into(),
    }
}

/// Build DAP `launch` arguments for a frontend or headless client.
pub fn build_launch_arguments(config: &SessionInitConfig) -> serde_json::Value {
    if wants_python_launch_hints(config.adapter_id.as_deref()) {
        return python_launch_arguments(&config.program);
    }

    let mut args = json!({ "program": config.program });
    if wants_rust_launch_hints(config.adapter_id.as_deref(), &config.program) {
        args["sourceLanguages"] = json!(["rust"]);
    }
    args
}

async fn read_until_event(
    client: &mut ControlClient,
    name: &str,
    timeout: Duration,
) -> Result<Message> {
    client.wait_for_event(name, timeout).await
}
