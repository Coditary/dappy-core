mod common;

use common::{ReplNdjsonSession, assert_ok, lock_tests};
use serde_json::json;

#[tokio::test]
async fn json_proto_covers_remaining_ops() {
    let _guard = lock_tests().await;
    let mut session = ReplNdjsonSession::spawn().await;
    let _ready = session.read_response().await;

    let ops = [
        json!({ "id": 1, "op": "step_in" }),
        json!({ "id": 2, "op": "step_out" }),
        json!({ "id": 3, "op": "step_back" }),
        json!({ "id": 4, "op": "reverse_continue" }),
        json!({ "id": 5, "op": "pause" }),
        json!({ "id": 6, "op": "breakpoints" }),
        json!({ "id": 7, "op": "set_variable", "name": "x", "value": "7" }),
        json!({ "id": 8, "op": "catch", "filter": "uncaught", "action": "add" }),
        json!({ "id": 9, "op": "exception_filters" }),
        json!({ "id": 10, "op": "catch", "action": "clear" }),
        json!({ "id": 11, "op": "skip_list" }),
        json!({ "id": 12, "op": "skip_add", "pattern": "/vendor/" }),
        json!({ "id": 13, "op": "skip_clear", "pattern": "/vendor/" }),
        json!({ "id": 14, "op": "context" }),
        json!({ "id": 15, "op": "state" }),
        json!({
            "id": 16,
            "op": "breakpoint",
            "path": "/fake/main.py",
            "line": 2,
            "action": "toggle"
        }),
        json!({
            "id": 17,
            "op": "trace",
            "path": "/fake/main.py",
            "line": 2,
            "log_message": "seen"
        }),
        json!({ "id": 18, "op": "variables", "variables_reference": 1 }),
        json!({ "id": 19, "op": "config" }),
        json!({
            "id": 20,
            "op": "read_memory",
            "memory_reference": "0x1000",
            "count": 16
        }),
        json!({
            "id": 21,
            "op": "write_memory",
            "memory_reference": "0x1000",
            "data": "48656C6C6F"
        }),
        json!({
            "id": 22,
            "op": "thread_snapshot",
            "include_stacks": true,
            "stack_depth": 3
        }),
        json!({ "id": 23, "op": "dap", "command": "threads", "arguments": {} }),
        json!({ "id": 24, "op": "quit" }),
    ];

    for op in ops {
        let response = session.send(&op).await;
        assert_ok(&response);
    }

    session.finish().await;
}
