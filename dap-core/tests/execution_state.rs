use dap_core::{ExecutionStateTracker, ExecutionStatus};
use dap_protocol::Message;

#[test]
fn execution_tracker_tracks_running_and_stopped() {
    let mut tracker = ExecutionStateTracker::new();
    assert_eq!(tracker.summary().state.status, ExecutionStatus::Unknown);

    tracker.apply_message(&Message::Event(dap_protocol::Event {
        seq: 1,
        event: "continued".into(),
        body: Some(serde_json::json!({ "threadId": 1, "allThreadsContinued": true })),
    }));
    assert_eq!(tracker.summary().state.status, ExecutionStatus::Running);

    tracker.apply_message(&Message::Event(dap_protocol::Event {
        seq: 2,
        event: "stopped".into(),
        body: Some(serde_json::json!({
            "reason": "breakpoint",
            "threadId": 1,
            "description": "paused"
        })),
    }));
    let summary = tracker.summary();
    assert_eq!(summary.state.status, ExecutionStatus::Stopped);
    assert_eq!(summary.state.stop_reason.as_deref(), Some("breakpoint"));
    assert_eq!(summary.state.thread_id, Some(1));
    assert_eq!(summary.version, 2);
}

#[test]
fn execution_tracker_ignores_unrelated_messages() {
    let mut tracker = ExecutionStateTracker::new();
    tracker.apply_message(&Message::Response(dap_protocol::Response {
        seq: 1,
        request_seq: 1,
        success: true,
        command: Some("threads".into()),
        message: None,
        body: Some(serde_json::json!({ "threads": [] })),
    }));
    assert_eq!(tracker.summary().state.status, ExecutionStatus::Unknown);
}
