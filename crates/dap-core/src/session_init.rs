use std::time::Duration;

use anyhow::{Context, Result};
use dap_plugin_api::{InitStep, PluginManifest, RecordEntry, TemplateContext, resolve_value};
use dap_protocol::Message;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::capabilities::AdapterCapabilities;
use crate::control_client::ControlClient;

const INIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Configuration for a headless DAP initialization sequence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInitConfig {
    pub program: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_id: Option<String>,
    #[serde(default)]
    pub client_id: String,
    #[serde(skip)]
    pub manifest: Option<PluginManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_overrides: Option<Value>,
}

impl SessionInitConfig {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            adapter_id: None,
            client_id: "dap-core".into(),
            manifest: None,
            launch_overrides: None,
        }
    }

    pub fn with_adapter_id(mut self, adapter_id: impl Into<String>) -> Self {
        self.adapter_id = Some(adapter_id.into());
        self
    }

    pub fn with_manifest(mut self, manifest: PluginManifest) -> Self {
        self.manifest = Some(manifest);
        self
    }

    pub fn with_launch_overrides(mut self, overrides: Value) -> Self {
        self.launch_overrides = Some(overrides);
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
    let request = config
        .manifest
        .as_ref()
        .map(|manifest| manifest.launch_request_name())
        .unwrap_or("launch");
    let launch_args = build_launch_arguments(config);
    run_debug_request(client, config, request, &launch_args).await
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

    let entry_location = if let Some(manifest) = config.manifest.as_ref() {
        if manifest.init_steps().is_empty() {
            run_default_init_sequence(client, request, arguments).await?;
            None
        } else {
            run_manifest_init_sequence(client, config, manifest, request, arguments).await?
        }
    } else {
        run_default_init_sequence(client, request, arguments).await?;
        None
    };

    let stopped = read_until_event(client, "stopped", INIT_TIMEOUT)
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
    if let Some(manifest) = config.manifest.as_ref() {
        return manifest
            .build_initialize_arguments(&config.client_id, config.adapter_id.as_deref());
    }

    json!({
        "clientID": config.client_id,
        "adapterID": config.adapter_id.clone().unwrap_or_else(|| "dap-proxy".into()),
        "linesStartAt1": true,
        "columnsStartAt1": true,
        "pathFormat": "path",
        "supportsRunInTerminalRequest": true,
    })
}

/// Build DAP `launch` arguments for a frontend or headless client.
pub fn build_launch_arguments(config: &SessionInitConfig) -> serde_json::Value {
    let mut args = if let Some(manifest) = config.manifest.as_ref() {
        manifest.build_launch_arguments(&config.program)
    } else {
        json!({ "program": config.program })
    };
    if let Some(overrides) = &config.launch_overrides {
        merge_launch_arguments(&mut args, overrides);
    }
    args
}

fn merge_launch_arguments(base: &mut Value, overrides: &Value) {
    let Value::Object(override_map) = overrides else {
        return;
    };
    let Value::Object(base_map) = base else {
        return;
    };
    for (key, value) in override_map {
        base_map.insert(key.clone(), value.clone());
    }
}

async fn run_default_init_sequence(
    client: &mut ControlClient,
    request: &str,
    arguments: &Value,
) -> Result<()> {
    client
        .dap_request_preserve_events(request, Some(arguments.clone()))
        .await
        .with_context(|| request.to_string())?;

    read_until_event(client, "initialized", INIT_TIMEOUT)
        .await
        .context("wait for initialized event")?;

    client
        .dap_request_preserve_events("configurationDone", Some(json!({})))
        .await
        .context("configurationDone")?;
    Ok(())
}

async fn run_manifest_init_sequence(
    client: &mut ControlClient,
    config: &SessionInitConfig,
    manifest: &PluginManifest,
    request: &str,
    launch_arguments: &Value,
) -> Result<Option<SessionEntryLocation>> {
    let ctx = manifest.template_context(&config.program);
    let mut entry_location = None;

    for step in manifest.init_steps() {
        if step.request.is_none() {
            if let Some(wait_for) = &step.wait_for_response {
                client
                    .wait_for_response_named(wait_for)
                    .await
                    .with_context(|| format!("wait for {wait_for} response"))?;
            }
            continue;
        }

        let command = step.request.as_deref().expect("checked");
        let arguments = step_arguments(step, request, launch_arguments, &ctx)?;

        if let Some(event_name) = &step.wait_for_event {
            let seq = client
                .send_dap_request_and_wait_for_event(
                    command,
                    arguments.clone(),
                    event_name,
                    INIT_TIMEOUT,
                )
                .await
                .with_context(|| format!("{command} waiting for {event_name}"))?;

            if let Some(record) = &step.record_entry {
                entry_location = Some(resolve_entry_location(record, &ctx)?);
            }

            if step.wait_for_response.as_deref() == Some(command) {
                client
                    .wait_for_response_seq(seq)
                    .await
                    .with_context(|| format!("wait for {command} response"))?;
            }
            continue;
        }

        client
            .dap_request_preserve_events(command, arguments.clone())
            .await
            .with_context(|| command.to_string())?;

        if let Some(record) = &step.record_entry {
            entry_location = Some(resolve_entry_location(record, &ctx)?);
        }

        if let Some(wait_for) = &step.wait_for_response {
            client
                .wait_for_response_named(wait_for)
                .await
                .with_context(|| format!("wait for {wait_for} response"))?;
        }
    }

    Ok(entry_location)
}

fn step_arguments(
    step: &InitStep,
    request: &str,
    launch_arguments: &Value,
    ctx: &TemplateContext,
) -> Result<Option<Value>> {
    if step.use_launch_arguments {
        return Ok(Some(launch_arguments.clone()));
    }

    if let Some(arguments) = &step.arguments {
        return Ok(resolve_value(arguments, ctx));
    }

    if step.request.as_deref() == Some(request) {
        return Ok(Some(launch_arguments.clone()));
    }

    Ok(Some(json!({})))
}

fn resolve_entry_location(
    record: &RecordEntry,
    ctx: &TemplateContext,
) -> Result<SessionEntryLocation> {
    let path = resolve_value(&json!(record.path), ctx)
        .and_then(|value| value.as_str().map(str::to_string))
        .with_context(|| format!("resolve entry path template {}", record.path))?;
    Ok(SessionEntryLocation {
        path,
        line: record.line,
        frame_id: Some(1),
    })
}

async fn read_until_event(
    client: &mut ControlClient,
    name: &str,
    timeout: Duration,
) -> Result<Message> {
    client.wait_for_event(name, timeout).await
}
