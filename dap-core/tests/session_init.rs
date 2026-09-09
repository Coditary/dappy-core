use dap_core::{Backend, ControlClient, MultiplexOptions, SessionInitConfig, run_session_init};
use dap_plugin_api::{AdapterSpawn, SpawnTransport};
use tokio::time::{Duration, sleep};

#[tokio::test]
async fn headless_session_init_over_control_client() {
    let adapter = adapter_bin();
    let backend = Backend::spawn(&AdapterSpawn {
        transport: SpawnTransport::Stdio,
        command: adapter.to_string_lossy().into_owned(),
        args: vec![],
    })
    .await
    .expect("spawn fake adapter");

    let proxy = dap_core::start_headless_multiplexed_proxy(
        backend.into_duplex(),
        MultiplexOptions {
            control_port: Some(0),
            ..Default::default()
        },
        None,
    )
    .await
    .expect("start headless mux proxy");

    // Let the control listener finish attaching before the first DAP request.
    sleep(Duration::from_millis(50)).await;

    let mut client = ControlClient::connect(proxy.control_port.unwrap())
        .await
        .expect("connect control");

    let result = run_session_init(
        &mut client,
        &SessionInitConfig::new("main.py").with_adapter_id("fake"),
    )
    .await
    .expect("session init");

    assert!(result.ready);
    assert_eq!(result.stop_reason.as_deref(), Some("entry"));
    assert_eq!(result.thread_id, Some(1));
    assert!(!result.capabilities.supports_conditional_breakpoints);

    proxy.join.abort();
}

#[tokio::test]
async fn execution_state_tracks_stopped_event() {
    use dap_core::{ExecutionStateTracker, ExecutionStatus};
    use dap_protocol::{Event, Message};

    let mut tracker = ExecutionStateTracker::new();
    tracker.apply_message(&Message::Event(Event {
        seq: 1,
        event: "stopped".into(),
        body: Some(serde_json::json!({
            "reason": "entry",
            "threadId": 1,
        })),
    }));

    let summary = tracker.summary();
    assert_eq!(summary.state.status, ExecutionStatus::Stopped);
    assert_eq!(summary.state.thread_id, Some(1));
    assert_eq!(summary.state.stop_reason.as_deref(), Some("entry"));
}

fn adapter_bin() -> std::path::PathBuf {
    std::env::var("CARGO_BIN_EXE_fake-dap-adapter")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let exe = std::env::current_exe().expect("current_exe");
            let debug_dir = exe.parent().expect("parent").parent().expect("debug");
            debug_dir.join("fake-dap-adapter")
        })
}
