mod common;

use common::{ReplNdjsonSession, assert_ok, lock_tests};
use serde_json::json;

#[tokio::test]
async fn repl_ndjson_ready_and_help() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let ready = session.read_response().await;
    assert_ok(&ready);
    assert_eq!(
        ready["result"]["event"].as_str(),
        Some("ready"),
        "first line should be ready event"
    );
    assert!(ready["result"]["context"]["thread_id"].is_number());

    let help = session
        .send(&json!({ "id": 1, "op": "help" }))
        .await;
    assert_ok(&help);
    assert_eq!(help["id"], 1);
    assert!(help["result"]["ops"].is_array());

    let quit = session
        .send(&json!({ "id": 2, "op": "quit" }))
        .await;
    assert_ok(&quit);
    assert_eq!(quit["result"]["disconnected"], true);

    session.finish().await;
}

#[tokio::test]
async fn repl_ndjson_demo_script() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let ready = session.read_response().await;
    assert_ok(&ready);

    let commands = [
        json!({ "id": 1, "op": "threads" }),
        json!({ "id": 2, "op": "stack" }),
        json!({ "id": 3, "op": "status" }),
        json!({
            "id": 4,
            "op": "breakpoint",
            "path": "/fake/main.py",
            "line": 1,
            "action": "add"
        }),
        json!({ "id": 5, "op": "step_over" }),
        json!({ "id": 6, "op": "scopes" }),
        json!({ "id": 7, "op": "locals" }),
        json!({ "id": 8, "op": "evaluate", "expression": "1 + 1" }),
        json!({ "id": 9, "op": "capabilities" }),
        json!({ "id": 10, "op": "clear", "path": "/fake/main.py", "line": 1 }),
        json!({ "id": 11, "op": "quit" }),
    ];

    for command in commands {
        let response = session.send(&command).await;
        assert_ok(&response);
        if let Some(id) = command.get("id") {
            assert_eq!(&response["id"], id);
        }
    }

    session.finish().await;
}

#[tokio::test]
async fn repl_breakpoint_reports_emulated_features() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let response = session
        .send(&json!({
            "id": 1,
            "op": "breakpoint",
            "path": "/fake/main.py",
            "line": 1,
            "action": "add",
            "condition": "x > 0",
            "hit_condition": "3",
            "log_message": "hit"
        }))
        .await;
    assert_ok(&response);

    let emulated = &response["result"]["emulated"];
    assert_eq!(emulated["condition"], true);
    assert_eq!(emulated["hit"], true);
    assert_eq!(emulated["log"], true);

    let _ = session.send(&json!({ "id": 2, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_hit_count_waits_until_threshold() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;
    let program = common::fixture_path("main.py").to_string_lossy().to_string();

    let _ = session
        .send(&json!({
            "id": 1,
            "op": "breakpoint",
            "path": program,
            "line": 51,
            "action": "add",
            "hit_condition": "2"
        }))
        .await;

    let sync = session.send(&json!({ "id": 2, "op": "sync" })).await;
    assert_ok(&sync);
    assert_eq!(sync["result"]["auto_continues"], 1);

    let _ = session.send(&json!({ "id": 3, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_sync_reports_execution_state() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let sync = session.send(&json!({ "id": 1, "op": "sync" })).await;
    assert_ok(&sync);
    assert!(sync["result"]["execution"].is_object());
    assert!(sync["result"]["stack"]["stackFrames"].is_array());
    assert_eq!(sync["result"]["auto_continues"], 0);

    let _ = session.send(&json!({ "id": 2, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_unknown_op_returns_error() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let response = session
        .send(&json!({ "id": 1, "op": "not_a_real_command" }))
        .await;
    assert_eq!(response.get("ok").and_then(|v| v.as_bool()), Some(false));
    assert!(response.get("error").and_then(|v| v.as_str()).is_some());

    let _ = session.send(&json!({ "id": 2, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_lists_breakpoints_and_sets_variable() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let _ = session
        .send(&json!({
            "id": 1,
            "op": "breakpoint",
            "path": "/fake/main.py",
            "line": 1,
            "action": "add"
        }))
        .await;

    let breakpoints = session.send(&json!({ "id": 2, "op": "breakpoints" })).await;
    assert_ok(&breakpoints);
    assert!(breakpoints["result"]["/fake/main.py"].is_array());

    let set_var = session
        .send(&json!({
            "id": 3,
            "op": "set_variable",
            "name": "x",
            "value": "99"
        }))
        .await;
    assert_ok(&set_var);
    assert_eq!(set_var["result"]["result"]["value"].as_str(), Some("99"));

    let _ = session.send(&json!({ "id": 4, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_exception_breakpoints_round_trip() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let filters = session.send(&json!({ "id": 1, "op": "exception_filters" })).await;
    assert_ok(&filters);
    assert!(filters["result"]["filters"].as_array().unwrap().len() >= 2);

    let catch = session
        .send(&json!({
            "id": 2,
            "op": "catch",
            "filter": "uncaught",
            "action": "add"
        }))
        .await;
    assert_ok(&catch);
    assert_eq!(catch["result"]["installed"][0]["filter"].as_str(), Some("uncaught"));

    let clear = session
        .send(&json!({ "id": 3, "op": "catch", "action": "clear" }))
        .await;
    assert_ok(&clear);
    assert_eq!(clear["result"]["installed"].as_array().unwrap().len(), 0);

    let _ = session.send(&json!({ "id": 4, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_show_displays_source_snippet() {
    let _guard = lock_tests().await;

    let dir = std::env::temp_dir().join(format!(
        "dap-repl-show-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let main_py = dir.join("main.py");
    std::fs::write(&main_py, "line1\nline2\nline3\nline4\nline5\n").expect("write main.py");

    let mut session =
        ReplNdjsonSession::spawn_with_program_in_dir(main_py.to_str().expect("path"), &dir).await;
    let _ready = session.read_response().await;

    let show = session.send(&json!({ "id": 1, "op": "show" })).await;
    assert_ok(&show);
    assert_eq!(show["result"]["source_available"], true);
    let display = show["result"]["display"].as_str().expect("display");
    assert!(display.contains("main.py:"));
    assert!(display.contains("in accumulate"));
    assert!(display.lines().any(|line| line.contains('>') && line.contains("line")));

    let _ = session.send(&json!({ "id": 2, "op": "quit" })).await;
    session.finish().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn repl_suppresses_stop_on_entry() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let ready = session.read_response().await;
    assert_ok(&ready);
    let stop_reason = ready["result"]["context"]["execution"]["stopReason"]
        .as_str()
        .unwrap_or("");
    assert_ne!(stop_reason, "entry");

    let _ = session.send(&json!({ "id": 1, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_client_side_watches_refresh_on_sync() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let add = session
        .send(&json!({ "id": 1, "op": "watch_add", "expression": "1 + 1" }))
        .await;
    assert_ok(&add);
    assert_eq!(add["result"]["expressions"][0].as_str(), Some("1 + 1"));

    let sync = session.send(&json!({ "id": 2, "op": "sync" })).await;
    assert_ok(&sync);
    let watches = sync["result"]["watches"].as_array().expect("watches");
    assert_eq!(watches.len(), 1);
    assert_eq!(watches[0]["expression"].as_str(), Some("1 + 1"));

    let _ = session.send(&json!({ "id": 3, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_breakpoint_matches_source_by_basename() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;
    let program = common::fixture_path("main.py").to_string_lossy().to_string();

    let _ = session
        .send(&json!({
            "id": 1,
            "op": "breakpoint",
            "path": "main.py",
            "line": 51,
            "action": "add",
            "hit_condition": "2"
        }))
        .await;

    let sync = session.send(&json!({ "id": 2, "op": "sync" })).await;
    assert_ok(&sync);
    assert_eq!(sync["result"]["auto_continues"], 1);

    let breakpoints = sync["result"]["breakpoints"]["merged"]["main.py"]
        .as_array()
        .expect("basename breakpoint");
    assert_eq!(breakpoints[0]["line"], 51);

    let _ = session
        .send(&json!({ "id": 3, "op": "clear", "path": program, "line": 51 }))
        .await;
    let _ = session.send(&json!({ "id": 4, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_exception_catch_with_condition_is_emulated_when_unsupported() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let catch = session
        .send(&json!({
            "id": 1,
            "op": "catch",
            "filter": "uncaught",
            "action": "add",
            "condition": "false"
        }))
        .await;
    assert_ok(&catch);
    assert_eq!(
        catch["result"]["installed"][0]["emulated"]["condition"],
        false
    );

    let _ = session.send(&json!({ "id": 2, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_function_breakpoint_resolves_definition() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let response = session
        .send(&json!({
            "id": 1,
            "op": "function_breakpoint",
            "name": "accumulate",
            "action": "add"
        }))
        .await;
    assert_ok(&response);
    assert_eq!(response["result"]["installed"][0]["name"].as_str(), Some("accumulate"));
    assert_eq!(response["result"]["installed"][0]["emulated"], true);
    assert!(response["result"]["installed"][0]["resolved_line"].is_number());

    let _ = session.send(&json!({ "id": 2, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_completions_returns_empty_when_unsupported() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let response = session
        .send(&json!({ "id": 1, "op": "completions", "expression": "acc" }))
        .await;
    assert_ok(&response);
    assert_eq!(response["result"]["supported"], false);
    assert_eq!(response["result"]["targets"].as_array().unwrap().len(), 0);

    let _ = session.send(&json!({ "id": 2, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_goto_targets_emulated_when_unsupported() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;
    let program = common::fixture_path("main.py").to_string_lossy().to_string();

    let response = session
        .send(&json!({ "id": 1, "op": "goto_targets", "path": program, "line": 50 }))
        .await;
    assert_ok(&response);
    assert_eq!(response["result"]["supported"], false);
    assert_eq!(response["result"]["emulated"], true);
    assert!(response["result"]["targets"].as_array().unwrap().len() >= 1);

    let _ = session.send(&json!({ "id": 2, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_data_watch_fires_on_value_change() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let _ = session
        .send(&json!({
            "id": 1,
            "op": "data_watch_add",
            "expression": "data_watch_marker"
        }))
        .await;

    let sync1 = session.send(&json!({ "id": 2, "op": "sync" })).await;
    assert_ok(&sync1);
    assert_eq!(sync1["result"]["data_watch_hits"].as_array().unwrap().len(), 0);

    let _ = session.send(&json!({ "id": 3, "op": "step_over" })).await;
    let sync2 = session.send(&json!({ "id": 4, "op": "sync" })).await;
    assert_ok(&sync2);
    assert_eq!(sync2["result"]["data_watch_hits"].as_array().unwrap().len(), 1);

    let _ = session.send(&json!({ "id": 5, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_restart_frame_emulated_via_goto() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let response = session
        .send(&json!({ "id": 1, "op": "restart_frame", "frame_id": 1 }))
        .await;
    assert_ok(&response);
    assert_eq!(response["result"]["method"].as_str(), Some("restart_frame_emulated"));
    assert_eq!(response["result"]["emulated"], true);

    let _ = session.send(&json!({ "id": 2, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_sync_includes_presentation() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let sync = session.send(&json!({ "id": 1, "op": "sync" })).await;
    assert_ok(&sync);
    assert!(sync["result"]["presentation"]["stack"]
        .as_str()
        .is_some_and(|text| text.contains("Stack trace")));
    assert!(sync["result"]["presentation"]["breakpoints"]
        .as_str()
        .is_some_and(|text| text.contains("Breakpoints")));

    let _ = session.send(&json!({ "id": 2, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_instruction_breakpoint_emulated_via_disassemble() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let response = session
        .send(&json!({
            "id": 1,
            "op": "instruction_breakpoint",
            "memory_reference": "0x1000",
            "offset": 0,
            "value": "true"
        }))
        .await;
    assert_ok(&response);
    assert_eq!(
        response["result"]["method"].as_str(),
        Some("instruction_breakpoint_emulated")
    );
    assert_eq!(response["result"]["emulated"], true);
    assert!(response["result"]["line"].is_number());

    let _ = session.send(&json!({ "id": 2, "op": "quit" })).await;
    session.finish().await;
}

#[tokio::test]
async fn repl_disassemble_returns_instructions() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let response = session
        .send(&json!({
            "id": 1,
            "op": "disassemble",
            "memory_reference": "0x1000",
            "offset": 0,
            "count": 1
        }))
        .await;
    assert_ok(&response);
    assert!(response["result"]["body"]["instructions"]
        .as_array()
        .is_some_and(|items| !items.is_empty()));

    let _ = session.send(&json!({ "id": 2, "op": "quit" })).await;
    session.finish().await;
}
