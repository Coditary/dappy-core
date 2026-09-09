use dap_core::{
    NavigateResult, VersionedExecutionState, format_evaluate, format_scopes, format_stack_trace,
    format_status, format_threads, format_variables,
};
use serde_json::Value;

pub fn print_json(value: &Value, json_mode: bool) {
    if json_mode {
        println!("{}", serde_json::to_string(value).expect("serialize json"));
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(value).expect("serialize json")
        );
    }
}

pub fn print_threads(value: &Value, json_mode: bool) {
    if json_mode {
        print_json(value, json_mode);
    } else {
        print!("{}", format_threads(value));
    }
}

pub fn print_stack_trace(value: &Value, json_mode: bool) {
    if json_mode {
        print_json(value, json_mode);
    } else {
        print!("{}", format_stack_trace(value));
    }
}

pub fn print_status(summary: &VersionedExecutionState, json_mode: bool) {
    if json_mode {
        let value = serde_json::to_value(summary).expect("serialize status");
        print_json(&value, json_mode);
    } else {
        print!("{}", format_status(summary));
    }
}

pub fn print_evaluate(value: &Value, json_mode: bool) {
    if json_mode {
        print_json(value, json_mode);
    } else {
        println!("{}", format_evaluate(value));
    }
}

pub fn print_scopes(value: &Value, json_mode: bool) {
    if json_mode {
        print_json(value, json_mode);
    } else {
        print!("{}", format_scopes(value));
    }
}

pub fn print_variables(value: &Value, json_mode: bool) {
    if json_mode {
        print_json(value, json_mode);
    } else {
        print!("{}", format_variables(value));
    }
}

pub fn print_navigate(result: &NavigateResult, json_mode: bool) {
    print_json(
        &serde_json::to_value(result).expect("serialize navigate result"),
        json_mode,
    );
}

#[cfg(test)]
mod output_tests {
    use super::*;
    use dap_core::{NavigateResult, NavigationType, VersionedExecutionState};
    use serde_json::json;

    #[test]
    fn print_json_modes() {
        let value = json!({"ok": true});
        print_json(&value, true);
        print_json(&value, false);
    }

    #[test]
    fn print_render_helpers() {
        let threads = json!({"threads": [{"id": 1, "name": "main"}]});
        print_threads(&threads, false);
        print_threads(&threads, true);

        let stack = json!({"stackFrames": [{"id": 1, "name": "main", "line": 1, "source": {"path": "a.py"}}]});
        print_stack_trace(&stack, false);
        print_stack_trace(&stack, true);

        let summary = VersionedExecutionState {
            version: 1,
            state: dap_core::ExecutionStateSummary {
                status: dap_core::ExecutionStatus::Stopped,
                stop_reason: Some("breakpoint".into()),
                thread_id: Some(1),
                description: Some("paused".into()),
            },
        };
        print_status(&summary, false);
        print_status(&summary, true);

        let eval = json!({"result": "42"});
        print_evaluate(&eval, false);
        print_evaluate(&eval, true);

        let scopes = json!({"scopes": [{"name": "Locals", "variablesReference": 1, "expensive": false}]});
        print_scopes(&scopes, false);
        print_scopes(&scopes, true);

        let vars = json!({"variables": [{"name": "x", "value": "1"}]});
        print_variables(&vars, false);
        print_variables(&vars, true);

        let nav = NavigateResult {
            navigation_type: NavigationType::StepOver,
            success: true,
            stop_reason: Some("step".into()),
            thread_id: Some(1),
        };
        print_navigate(&nav, false);
        print_navigate(&nav, true);
    }
}
