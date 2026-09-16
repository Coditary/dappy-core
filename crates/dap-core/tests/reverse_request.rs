use dap_core::handle_reverse_request;
use dap_protocol::{Message, Request};

#[test]
fn reverse_request_start_debugging_fails_closed_without_child_sessions() {
    let message = Message::Request(Request {
        seq: 7,
        command: "startDebugging".into(),
        arguments: Some(serde_json::json!({
            "request": "launch",
            "configuration": {}
        })),
    });
    let response = handle_reverse_request(&message).expect("should handle startDebugging");
    match response {
        Message::Response(resp) => {
            assert!(!resp.success);
            assert_eq!(resp.request_seq, 7);
        }
        other => panic!("expected response, got {:?}", other),
    }
}

#[test]
fn reverse_request_run_in_terminal_spawns_command() {
    let message = Message::Request(Request {
        seq: 3,
        command: "runInTerminal".into(),
        arguments: Some(serde_json::json!({
            "args": ["/bin/true"],
            "cwd": "/",
        })),
    });
    let response = handle_reverse_request(&message).expect("should handle runInTerminal");
    match response {
        Message::Response(resp) => {
            assert!(resp.success);
            assert_eq!(resp.request_seq, 3);
            let body = resp.body.expect("runInTerminal body");
            assert_eq!(body["captured"], true);
            assert!(
                body["logPath"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("3.log"))
            );
        }
        other => panic!("expected response, got {:?}", other),
    }
}

#[test]
fn reverse_request_ignores_other_commands() {
    let message = Message::Request(Request {
        seq: 1,
        command: "completions".into(),
        arguments: None,
    });
    assert!(handle_reverse_request(&message).is_none());
}
