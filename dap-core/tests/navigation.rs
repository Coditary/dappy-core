use dap_core::{
    NavigationType, format_evaluate, format_scopes, format_stack_trace, format_threads,
    format_variables,
};
use serde_json::json;

#[test]
fn format_threads_renders_list() {
    let body = json!({
        "threads": [
            { "id": 1, "name": "main" },
            { "id": 2, "name": "worker" }
        ]
    });
    let text = format_threads(&body);
    assert!(text.contains("[1] main"));
    assert!(text.contains("[2] worker"));
}

#[test]
fn format_stack_trace_renders_frames() {
    let body = json!({
        "stackFrames": [{
            "id": 10,
            "name": "foo",
            "line": 5,
            "source": { "path": "/src/foo.rs" }
        }]
    });
    let text = format_stack_trace(&body);
    assert!(text.contains("foo"));
    assert!(text.contains("foo.rs:5"));
}

#[test]
fn format_evaluate_returns_result() {
    let body = json!({ "result": "42", "variablesReference": 0 });
    assert_eq!(format_evaluate(&body), "42");
}

#[test]
fn format_scopes_and_variables() {
    let scopes = json!({
        "scopes": [{ "name": "Locals", "variablesReference": 1, "expensive": false }]
    });
    assert!(format_scopes(&scopes).contains("Locals"));

    let vars = json!({
        "variables": [{ "name": "x", "value": "1", "type": "int" }]
    });
    assert!(format_variables(&vars).contains("x"));
    assert!(format_variables(&vars).contains("= 1"));
}

#[test]
fn navigation_type_parses_aliases() {
    assert_eq!("step-over".parse::<NavigationType>().unwrap(), NavigationType::StepOver);
    assert_eq!("next".parse::<NavigationType>().unwrap(), NavigationType::StepOver);
    assert_eq!(
        "reverse_continue".parse::<NavigationType>().unwrap(),
        NavigationType::ReverseContinue
    );
}

#[test]
fn format_breakpoints_and_exception_helpers() {
    use dap_core::{
        format_breakpoints, format_exception_breakpoints, format_exception_filters, format_status,
    };
    use dap_core::{ExecutionStateSummary, ExecutionStatus, VersionedExecutionState};

    let empty = json!({});
    assert!(format_breakpoints(&empty).contains("(none)"));

    let breakpoints = json!({
        "/src/main.py": [
            { "line": 10, "condition": "x > 0", "hit_condition": "3", "log_message": "here" }
        ]
    });
    let text = format_breakpoints(&breakpoints);
    assert!(text.contains("/src/main.py:10"));
    assert!(text.contains("if x > 0"));

    let installed = vec![json!({"filter": "uncaught", "condition": "err != nil"})];
    assert!(format_exception_breakpoints(&installed).contains("uncaught if err != nil"));

    let filters = vec![json!({"filter": "raised", "label": "Raised", "default": false})];
    assert!(format_exception_filters(&filters).contains("raised"));
    assert!(format_exception_breakpoints(&[]).contains("(none)"));
    assert!(format_exception_filters(&[]).contains("(none advertised)"));

    let summary = VersionedExecutionState {
        version: 1,
        state: ExecutionStateSummary {
            status: ExecutionStatus::Stopped,
            stop_reason: Some("breakpoint".into()),
            thread_id: Some(1),
            description: Some("paused".into()),
        },
    };
    assert!(format_status(&summary).contains("Stopped"));
}
