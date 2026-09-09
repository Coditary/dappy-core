mod profile;
mod resolve;

use std::sync::atomic::{AtomicU32, Ordering};

use dap_protocol::{Message, Request, Response};
use tracing::{info, warn};

pub use profile::{ChildSessionConfig, ChildSessionProfile, ParentBackendKind};
pub use resolve::{
    ChildBackendPlan, ChildSpawnPlan, ChildSpawnResult, ParentSessionContext, StartDebuggingArgs,
    resolve_child_spawn, strip_emit_start_debugging,
};

/// Abstraction over spawning a child `dap-proxy` process.
#[async_trait::async_trait]
pub trait ChildSessionSpawner: Send + Sync {
    async fn spawn(&self, plan: ChildSpawnPlan) -> anyhow::Result<ChildSpawnResult>;
}

/// Handle a `startDebugging` reverse request, spawning a child when configured.
pub async fn handle_start_debugging(
    request: &Request,
    config: &ChildSessionConfig,
    remaining_depth: u32,
    parent: &ParentSessionContext,
    spawner: &dyn ChildSessionSpawner,
    active_children: &AtomicU32,
) -> Message {
    let args = request
        .arguments
        .as_ref()
        .and_then(StartDebuggingArgs::from_value);

    let args = match args {
        Some(args) => args,
        None => return fail_reverse_response(request, "invalid startDebugging arguments"),
    };

    let plan = match resolve_child_spawn(
        config,
        remaining_depth,
        active_children.load(Ordering::Relaxed),
        parent,
        &args,
    ) {
        Ok(plan) => plan,
        Err(message) => {
            warn!(message, "declining startDebugging");
            return fail_reverse_response(request, &message);
        }
    };

    match spawner.spawn(plan).await {
        Ok(result) => {
            active_children.fetch_add(1, Ordering::Relaxed);
            info!(
                control_port = result.control_port,
                instance_id = %result.instance_id,
                "spawned child debug session"
            );
            success_reverse_response(request)
        }
        Err(err) => {
            warn!(error = %err, "failed to spawn child session");
            fail_reverse_response(request, &format!("failed to spawn child session: {err}"))
        }
    }
}

/// Decline reverse requests that this headless client does not handle.
pub fn decline_reverse_request(request: &Request) -> Message {
    fail_reverse_response(
        request,
        &format!("reverse request {:?} is not supported", request.command),
    )
}

fn success_reverse_response(request: &Request) -> Message {
    Message::Response(Response {
        seq: 0,
        request_seq: request.seq,
        success: true,
        command: Some("startDebugging".into()),
        message: None,
        body: None,
    })
}

fn fail_reverse_response(request: &Request, message: &str) -> Message {
    Message::Response(Response {
        seq: 0,
        request_seq: request.seq,
        success: false,
        command: Some(request.command.clone()),
        message: Some(message.to_string()),
        body: None,
    })
}

#[cfg(test)]
mod handler_tests {
    use super::*;
    use dap_protocol::Message;
    use serde_json::json;

    struct MockSpawner {
        ok: bool,
    }

    #[async_trait::async_trait]
    impl ChildSessionSpawner for MockSpawner {
        async fn spawn(&self, _plan: ChildSpawnPlan) -> anyhow::Result<ChildSpawnResult> {
            if self.ok {
                Ok(ChildSpawnResult {
                    control_port: 42,
                    instance_id: "child-1".into(),
                })
            } else {
                anyhow::bail!("spawn failed")
            }
        }
    }

    fn fake_config() -> ChildSessionConfig {
        ChildSessionConfig {
            auto_spawn: true,
            max_children: 2,
            max_depth: 1,
            profile: ChildSessionProfile::fake_preset(),
        }
    }

    fn stdio_parent() -> ParentSessionContext {
        ParentSessionContext {
            backend: ParentBackendKind::Stdio,
            adapter_cmd: vec!["fake".into()],
            tcp_endpoint: None,
        }
    }

    #[tokio::test]
    async fn handle_start_debugging_spawns_child_on_success() {
        let request = Request {
            seq: 3,
            command: "startDebugging".into(),
            arguments: Some(json!({
                "request": "launch",
                "configuration": {}
            })),
        };
        let active = AtomicU32::new(0);
        let response = handle_start_debugging(
            &request,
            &fake_config(),
            1,
            &stdio_parent(),
            &MockSpawner { ok: true },
            &active,
        )
        .await;
        match response {
            Message::Response(resp) => assert!(resp.success),
            other => panic!("unexpected {:?}", other),
        }
        assert_eq!(active.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn handle_start_debugging_rejects_invalid_arguments() {
        let request = Request {
            seq: 5,
            command: "startDebugging".into(),
            arguments: None,
        };
        let response = handle_start_debugging(
            &request,
            &ChildSessionConfig::default(),
            1,
            &stdio_parent(),
            &MockSpawner { ok: true },
            &AtomicU32::new(0),
        )
        .await;
        match response {
            Message::Response(resp) => assert!(!resp.success),
            other => panic!("unexpected {:?}", other),
        }
    }
}
