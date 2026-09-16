use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use dap_plugin_api::ResolvedTargetSpawn;
use serde_json::{Value, json};
use tokio::process::Child;
use tokio::time::sleep;

use crate::engine::DapEngine;

/// RSP target process plus attach fields for gdb-remote.
pub struct PreparedRspTarget {
    pub process: Option<Child>,
    pub attach_arguments: Value,
}

/// Resolve a `target.yaml` manifest, optionally spawn its backend, and build attach args.
pub async fn prepare_rsp_target(
    engine: &DapEngine,
    target_id: &str,
    program: &str,
) -> Result<PreparedRspTarget> {
    let manifest = engine
        .target_registry
        .get(target_id)
        .with_context(|| format!("unknown RSP target `{target_id}`"))?;
    let profile = manifest.resolve(program).map_err(anyhow::Error::from)?;
    let process = if let Some(spawn) = &profile.spawn {
        Some(spawn_rsp_target(spawn, &profile.host, profile.port).await?)
    } else {
        None
    };
    Ok(PreparedRspTarget {
        process,
        attach_arguments: profile.attach_arguments(),
    })
}

pub async fn spawn_rsp_target(
    spawn: &ResolvedTargetSpawn,
    host: &str,
    port: u16,
) -> Result<Child> {
    tracing::info!(
        command = %spawn.command,
        args = ?spawn.args,
        "starting RSP target"
    );
    let child = tokio::process::Command::new(&spawn.command)
        .args(&spawn.args)
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawn RSP target `{}`", spawn.command))?;
    wait_for_tcp(host, port, Duration::from_secs(60))
        .await
        .with_context(|| format!("wait for RSP endpoint at {host}:{port}"))?;
    Ok(child)
}

/// Fill missing attach/launch fields from target defaults; client values win on conflict.
pub fn merge_default_attach_arguments(client: &mut Value, defaults: &Value) {
    let Value::Object(defaults_map) = defaults else {
        return;
    };
    match client {
        Value::Object(client_map) => {
            for (key, value) in defaults_map {
                client_map.entry(key.clone()).or_insert(value.clone());
            }
        }
        Value::Null => {
            *client = defaults.clone();
        }
        _ => {
            let mut merged = defaults_map.clone();
            merged.insert("extra".into(), client.clone());
            *client = Value::Object(merged);
        }
    }
}

/// Merge attach defaults into a DAP request when forwarding to the adapter.
pub fn merge_debug_request_arguments(message: &mut dap_protocol::Message, defaults: &Value) {
    let dap_protocol::Message::Request(request) = message else {
        return;
    };
    if request.command != "attach" && request.command != "launch" {
        return;
    }
    let args = request.arguments.get_or_insert_with(|| json!({}));
    merge_default_attach_arguments(args, defaults);
}

async fn wait_for_tcp(host: &str, port: u16, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let addr = format!("{host}:{port}");
    while Instant::now() < deadline {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            return Ok(());
        }
        sleep(Duration::from_millis(100)).await;
    }
    bail!("timed out waiting for RSP endpoint {addr}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use dap_protocol::{Message, Request};
    use serde_json::json;

    #[test]
    fn merge_default_attach_arguments_preserves_client_values() {
        let mut client = json!({
            "host": "10.0.0.5",
            "program": "/tmp/app"
        });
        let defaults = json!({
            "host": "127.0.0.1",
            "port": 1234,
            "program": "/other/app",
            "stopAtEntry": true
        });
        merge_default_attach_arguments(&mut client, &defaults);
        assert_eq!(client["host"], "10.0.0.5");
        assert_eq!(client["port"], 1234);
        assert_eq!(client["program"], "/tmp/app");
        assert_eq!(client["stopAtEntry"], true);
    }

    #[test]
    fn merge_debug_request_arguments_updates_attach_body() {
        let mut message = Message::Request(Request {
            seq: 2,
            command: "attach".into(),
            arguments: Some(json!({ "program": "/tmp/app" })),
        });
        merge_debug_request_arguments(
            &mut message,
            &json!({ "host": "127.0.0.1", "port": 2331 }),
        );
        let Message::Request(request) = &message else {
            panic!("expected request");
        };
        let args = request.arguments.as_ref().expect("attach args");
        assert_eq!(args["host"], "127.0.0.1");
        assert_eq!(args["port"], 2331);
        assert_eq!(args["program"], "/tmp/app");
    }
}
