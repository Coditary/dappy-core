use std::path::Path;

use serde_json::Value;

use crate::stack_filter::is_hidden_stack_frame;
use crate::terminal_style::TerminalStyle;

/// Options for rendering stack traces in the REPL.
#[derive(Debug, Clone, Copy)]
pub struct StackTraceOptions {
    pub full: bool,
    pub style: Option<TerminalStyle>,
}

impl StackTraceOptions {
    pub fn user() -> Self {
        Self {
            full: false,
            style: Some(TerminalStyle::detect()),
        }
    }

    pub fn all() -> Self {
        Self {
            full: true,
            style: None,
        }
    }
}

/// Render threads JSON as human-readable text.
pub fn format_threads(body: &Value) -> String {
    let mut out = String::from("Threads:\n");
    match body.get("threads").and_then(Value::as_array) {
        Some(threads) if !threads.is_empty() => {
            for thread in threads {
                let id = thread.get("id").and_then(Value::as_i64).unwrap_or(-1);
                let name = thread
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                out.push_str(&format!("  [{id}] {name}\n"));
            }
        }
        _ => out.push_str("  (none)\n"),
    }
    out
}

/// Render stack trace JSON as human-readable text (all frames, no styling).
pub fn format_stack_trace(body: &Value) -> String {
    format_stack_trace_with_options(body, StackTraceOptions::all())
}

/// Render stack trace with optional frame filtering and styling.
pub fn format_stack_trace_with_options(body: &Value, options: StackTraceOptions) -> String {
    let style = options.style.unwrap_or(TerminalStyle { enabled: false });
    let mut out = String::from("Stack trace:\n");
    match body.get("stackFrames").and_then(Value::as_array) {
        Some(frames) if !frames.is_empty() => {
            let mut shown = 0usize;
            let mut hidden = 0usize;
            for frame in frames.iter() {
                let name = frame.get("name").and_then(Value::as_str).unwrap_or("?");
                let line = frame.get("line").and_then(Value::as_i64).unwrap_or(0);
                let path = frame
                    .get("source")
                    .and_then(|s| s.get("path"))
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                if is_hidden_stack_frame(path, options.full) {
                    hidden += 1;
                    continue;
                }
                let frame_id = frame.get("id").and_then(Value::as_i64).unwrap_or(-1);
                let location = format!("{}:{}", short_filename(path), line);
                let frame_label = style.bold(&format!("#{}", shown));
                let location_label = style.cyan(&location);
                out.push_str(&format!(
                    "  {frame_label} [{frame_id}] {name} ({location_label})\n",
                    frame_label = frame_label,
                    frame_id = frame_id,
                    name = name,
                    location_label = location_label,
                ));
                shown += 1;
            }
            if hidden > 0 {
                let note = format!(
                    "  ({hidden} runtime frame{s} hidden; use `bt full` to show all)",
                    hidden = hidden,
                    s = if hidden == 1 { "" } else { "s" },
                );
                out.push_str(&style.dim(&note));
                out.push('\n');
            }
            if shown == 0 {
                out.push_str("  (no user frames; try `bt full`)\n");
            }
        }
        _ => out.push_str("  (no frames)\n"),
    }
    out
}

/// Compact location string from the top stack frame, e.g. `main.py:75 in validate_config`.
pub fn format_stop_location(stack: &Value) -> Option<String> {
    let frame = stack
        .get("stackFrames")
        .and_then(Value::as_array)
        .and_then(|frames| frames.first())?;
    let path = frame
        .get("source")
        .and_then(|s| s.get("path"))
        .and_then(Value::as_str)?;
    let line = frame.get("line").and_then(Value::as_i64)?;
    let name = frame.get("name").and_then(Value::as_str).unwrap_or("?");
    Some(format!(
        "{}:{} in {}",
        short_filename(path),
        line,
        name
    ))
}

/// One-line status after navigation (step/continue).
pub fn format_navigate_status(nav: &Value, stack: &Value, style: Option<TerminalStyle>) -> String {
    let style = style.unwrap_or(TerminalStyle { enabled: false });
    let nav_type = nav
        .get("navigationType")
        .or_else(|| nav.get("navigation_type"))
        .and_then(Value::as_str)
        .unwrap_or("?");
    let reason = nav
        .get("stopReason")
        .or_else(|| nav.get("stop_reason"))
        .and_then(Value::as_str)
        .unwrap_or("?");
    let thread = nav
        .get("threadId")
        .or_else(|| nav.get("thread_id"))
        .and_then(Value::as_i64)
        .unwrap_or(0);

    let arrow = style.green("→");
    let location = format_stop_location(stack)
        .map(|loc| style.cyan(&loc))
        .unwrap_or_else(|| style.dim("?"));
    let meta = style.dim(&format!("({nav_type}, {reason}, thread {thread})"));
    format!("{arrow} {location} {meta}")
}

/// Render execution status summary.
pub fn format_status(summary: &crate::execution_state::VersionedExecutionState) -> String {
    let state = &summary.state;
    let mut out = format!("Status: {:?}\n", state.status);
    if let Some(reason) = &state.stop_reason {
        out.push_str(&format!("Stop reason: {reason}\n"));
    }
    if let Some(tid) = state.thread_id {
        out.push_str(&format!("Thread: {tid}\n"));
    }
    if let Some(desc) = &state.description {
        out.push_str(&format!("Description: {desc}\n"));
    }
    out
}

/// Render evaluate response body.
pub fn format_evaluate(body: &Value) -> String {
    if let Some(result) = body.get("result").and_then(Value::as_str) {
        return result.to_string();
    }
    body.to_string()
}

/// Render evaluate response with optional type annotation (for `pp`).
pub fn format_evaluate_pretty(body: &Value, style: Option<TerminalStyle>) -> String {
    let style = style.unwrap_or(TerminalStyle { enabled: false });
    let result = format_evaluate(body);
    let type_name = body
        .get("type")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty());
    match type_name {
        Some(type_name) => format!("{} {}", result, style.dim(&format!("({type_name})"))),
        None => result,
    }
}

/// Render scopes list.
pub fn format_scopes(body: &Value) -> String {
    let mut out = String::from("Scopes:\n");
    match body.get("scopes").and_then(Value::as_array) {
        Some(scopes) if !scopes.is_empty() => {
            for scope in scopes {
                let name = scope.get("name").and_then(Value::as_str).unwrap_or("?");
                let variables = scope
                    .get("variablesReference")
                    .and_then(Value::as_i64)
                    .unwrap_or(-1);
                let expensive = scope
                    .get("expensive")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                out.push_str(&format!(
                    "  {name} (variablesReference={variables}, expensive={expensive})\n"
                ));
            }
        }
        _ => out.push_str("  (none)\n"),
    }
    out
}

/// Render variables list.
pub fn format_variables(body: &Value) -> String {
    format_variables_table(body, None)
}

/// Render locals/variables as an aligned table.
pub fn format_variables_table(body: &Value, style: Option<TerminalStyle>) -> String {
    let style = style.unwrap_or(TerminalStyle { enabled: false });
    let mut out = String::from("Locals:\n");
    match body.get("variables").and_then(Value::as_array) {
        Some(vars) if !vars.is_empty() => {
            let name_width = vars
                .iter()
                .filter_map(|var| var.get("name").and_then(Value::as_str))
                .map(str::len)
                .max()
                .unwrap_or(4)
                .max(4);
            for var in vars {
                let name = var.get("name").and_then(Value::as_str).unwrap_or("?");
                let value = var.get("value").and_then(Value::as_str).unwrap_or("?");
                let type_name = var.get("type").and_then(Value::as_str).unwrap_or("");
                let name_col = style.bold(&format!("{name:<name_width$}"));
                if type_name.is_empty() {
                    out.push_str(&format!("  {name_col} = {value}\n"));
                } else {
                    let type_col = style.dim(&format!("({type_name})"));
                    out.push_str(&format!("  {name_col} = {value} {type_col}\n"));
                }
            }
        }
        _ => out.push_str("  (none)\n"),
    }
    out
}

/// Render tracked source breakpoints (REPL snapshot JSON).
pub fn format_breakpoints(body: &Value) -> String {
    format_breakpoints_table(body, false, None)
}

/// Render breakpoints as a numbered gdb-style table.
pub fn format_breakpoints_table(
    body: &Value,
    numbered: bool,
    style: Option<TerminalStyle>,
) -> String {
    let style = style.unwrap_or(TerminalStyle { enabled: false });
    let mut out = String::from("Breakpoints:\n");
    let Some(files) = body.as_object() else {
        out.push_str("  (none)\n");
        return out;
    };
    if files.is_empty() {
        out.push_str("  (none)\n");
        return out;
    }

    let mut number = 1usize;
    for (path, entries) in files {
        match entries.as_array() {
            Some(items) if !items.is_empty() => {
                for entry in items {
                    let line = entry.get("line").and_then(Value::as_i64).unwrap_or(0);
                    let mut extras = Vec::new();
                    if let Some(condition) = entry.get("condition").and_then(Value::as_str) {
                        extras.push(format!("if {condition}"));
                    }
                    if let Some(hit) = entry.get("hit_condition").and_then(Value::as_str) {
                        extras.push(format!("hit {hit}"));
                    }
                    if let Some(log) = entry.get("log_message").and_then(Value::as_str) {
                        extras.push(format!("log {log}"));
                    }
                    let extra = if extras.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", extras.join(" "))
                    };
                    let location = style.cyan(&format!("{path}:{line}"));
                    if numbered {
                        let num = style.yellow(&format!("{number}"));
                        out.push_str(&format!("  {num}: {location}{extra}\n"));
                        number += 1;
                    } else {
                        out.push_str(&format!("  {location}{extra}\n"));
                    }
                }
            }
            _ => out.push_str(&format!("  {path}: (none)\n")),
        }
    }
    out
}

/// Render installed exception breakpoint filters.
pub fn format_exception_breakpoints(entries: &[Value]) -> String {
    let mut out = String::from("Exception breakpoints:\n");
    if entries.is_empty() {
        out.push_str("  (none)\n");
        return out;
    }
    for entry in entries {
        let filter = entry.get("filter").and_then(Value::as_str).unwrap_or("?");
        match entry.get("condition").and_then(Value::as_str) {
            Some(condition) => out.push_str(&format!("  {filter} if {condition}\n")),
            None => out.push_str(&format!("  {filter}\n")),
        }
    }
    out
}

/// Render client-side watch values.
pub fn format_watches(watches: &Value) -> String {
    let mut out = String::from("Watches:\n");
    match watches.as_array() {
        Some(items) if !items.is_empty() => {
            for watch in items {
                let expression = watch
                    .get("expression")
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                let result = watch
                    .get("result")
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                out.push_str(&format!("  {expression} = {result}\n"));
            }
        }
        _ => out.push_str("  (none)\n"),
    }
    out
}

/// Render emulated data watches.
pub fn format_data_watches(entries: &Value) -> String {
    let mut out = String::from("Data watches:\n");
    match entries.as_array() {
        Some(items) if !items.is_empty() => {
            for entry in items {
                let expression = entry
                    .get("expression")
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                let baseline = entry
                    .get("baseline")
                    .and_then(Value::as_str)
                    .map(|value| format!(" baseline={value}"))
                    .unwrap_or_default();
                out.push_str(&format!("  {expression}{baseline}\n"));
            }
        }
        _ => out.push_str("  (none)\n"),
    }
    out
}

/// Build formatted presentation strings for sync/navigate responses.
pub fn build_sync_presentation(
    execution: &crate::execution_state::VersionedExecutionState,
    stack: &Value,
    watches: &Value,
    breakpoints: &Value,
    data_watches: &Value,
) -> Value {
    let stack_text = format_stack_trace_with_options(stack, StackTraceOptions::user());
    let status_text = format_status(execution);
    let watches_text = format_watches(watches);
    let breakpoints_text = format_breakpoints_table(
        breakpoints
            .get("merged")
            .unwrap_or(breakpoints),
        true,
        None,
    );
    let data_watches_text = format_data_watches(data_watches);
    serde_json::json!({
        "status": status_text,
        "stack": stack_text,
        "watches": watches_text,
        "breakpoints": breakpoints_text,
        "data_watches": data_watches_text,
    })
}

/// Render available exception filters from adapter capabilities.
pub fn format_exception_filters(filters: &[Value]) -> String {
    let mut out = String::from("Exception filters:\n");
    if filters.is_empty() {
        out.push_str("  (none advertised)\n");
        return out;
    }
    for filter in filters {
        let id = filter.get("filter").and_then(Value::as_str).unwrap_or("?");
        let label = filter.get("label").and_then(Value::as_str).unwrap_or(id);
        let default = filter
            .get("default")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        out.push_str(&format!("  {id} ({label}) default={default}\n"));
    }
    out
}

fn short_filename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stack_filter_hides_runtime_frames() {
        let body = json!({
            "stackFrames": [
                {
                    "id": 1,
                    "name": "runpy",
                    "line": 10,
                    "source": { "path": "/usr/lib/python3.12/runpy.py" }
                },
                {
                    "id": 2,
                    "name": "main",
                    "line": 5,
                    "source": { "path": "/project/main.py" }
                }
            ]
        });
        let filtered = format_stack_trace_with_options(&body, StackTraceOptions::user());
        assert!(!filtered.contains("runpy"));
        assert!(filtered.contains("main"));
        assert!(filtered.contains("hidden"));

        let full = format_stack_trace_with_options(
            &body,
            StackTraceOptions {
                full: true,
                style: None,
            },
        );
        assert!(full.contains("runpy"));
    }

    #[test]
    fn navigate_status_includes_location() {
        let nav = json!({ "navigationType": "step", "stopReason": "step", "threadId": 1 });
        let stack = json!({
            "stackFrames": [{
                "id": 1,
                "name": "validate_config",
                "line": 75,
                "source": { "path": "/tmp/main.py" }
            }]
        });
        let text = format_navigate_status(&nav, &stack, None);
        assert!(text.contains("main.py:75"));
        assert!(text.contains("validate_config"));
    }
}
