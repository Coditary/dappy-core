use dap_core::{
    ChildSessionConfig, ChildSessionProfile, ChildSessionSpawner, ChildSpawnPlan, ChildSpawnResult,
    ParentBackendKind, ParentSessionContext, StartDebuggingArgs, decline_reverse_request,
    handle_start_debugging, resolve_child_spawn,
};
use dap_protocol::{Message, Request};
use std::sync::atomic::AtomicU32;

struct OkSpawner;

#[async_trait::async_trait]
impl ChildSessionSpawner for OkSpawner {
    async fn spawn(&self, _plan: ChildSpawnPlan) -> anyhow::Result<ChildSpawnResult> {
        Ok(ChildSpawnResult {
            control_port: 9,
            instance_id: "child".into(),
        })
    }
}

fn stdio_parent(cmd: &[&str]) -> ParentSessionContext {
    ParentSessionContext {
        backend: ParentBackendKind::Stdio,
        adapter_cmd: cmd.iter().map(|s| s.to_string()).collect(),
        tcp_endpoint: None,
    }
}

#[test]
fn debugpy_profile_resolves_attach_tcp_plan() {
    let config = ChildSessionConfig {
        auto_spawn: true,
        max_children: 1,
        max_depth: 1,
        profile: ChildSessionProfile::debugpy_preset(),
    };
    let plan = resolve_child_spawn(
        &config,
        1,
        0,
        &stdio_parent(&["debugpy"]),
        &StartDebuggingArgs {
            request: "attach".into(),
            configuration: serde_json::json!({
                "connect": { "host": "127.0.0.1", "port": 9000 }
            }),
        },
    )
    .expect("plan");
    assert_eq!(plan.debug_request, "attach");
}

#[test]
fn decline_reverse_request_returns_failure_response() {
    let request = Request {
        seq: 2,
        command: "runInTerminal".into(),
        arguments: None,
    };
    let response = decline_reverse_request(&request);
    match response {
        Message::Response(resp) => {
            assert!(!resp.success);
            assert_eq!(resp.request_seq, 2);
        }
        other => panic!("unexpected {:?}", other),
    }
}

#[tokio::test]
async fn handle_start_debugging_integration() {
    let request = Request {
        seq: 1,
        command: "startDebugging".into(),
        arguments: Some(serde_json::json!({
            "request": "launch",
            "configuration": {}
        })),
    };
    let config = ChildSessionConfig {
        auto_spawn: true,
        max_children: 1,
        max_depth: 1,
        profile: ChildSessionProfile::fake_preset(),
    };
    let response = handle_start_debugging(
        &request,
        &config,
        1,
        &stdio_parent(&["adapter"]),
        &OkSpawner,
        &AtomicU32::new(0),
    )
    .await;
    match response {
        Message::Response(resp) => assert!(resp.success),
        other => panic!("unexpected {:?}", other),
    }
}

#[test]
fn child_profile_preset_deserializes_from_string() {
    let profile: ChildSessionProfile =
        serde_json::from_value(serde_json::json!("debugpy")).expect("preset");
    assert_eq!(profile, ChildSessionProfile::debugpy_preset());
}
