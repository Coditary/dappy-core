use anyhow::{Context, Result};
use dap_core::NavigationType;
use serde::Deserialize;
use serde_json::{Value, json};

use super::breakpoint::{BreakpointOptions, BreakpointRequest};
use super::context::{
    BreakpointAction, ExceptionBreakpointAction, FunctionBreakpointAction, JsonResponse, ReplContext,
};

pub fn ready_event(ctx: &ReplContext) -> Value {
    json!({
        "event": "ready",
        "context": ctx.ready_state(),
    })
}

#[derive(Debug, Default, Deserialize)]
pub struct JsonRequest {
    #[serde(default)]
    pub id: Option<Value>,
    pub op: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub line: Option<i64>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub hit_condition: Option<String>,
    #[serde(default)]
    pub log_message: Option<String>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub thread_id: Option<i64>,
    #[serde(default)]
    pub frame_id: Option<i64>,
    #[serde(default)]
    pub expression: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub filter: Option<String>,
    #[serde(default)]
    pub variables_reference: Option<i64>,
    #[serde(default)]
    pub context_lines: Option<u32>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub arguments: Option<Value>,
    #[serde(default)]
    pub memory_reference: Option<String>,
    #[serde(default)]
    pub count: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub include_stacks: Option<bool>,
    #[serde(default)]
    pub stack_depth: Option<i64>,
    #[serde(default)]
    pub max_threads: Option<i64>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub column: Option<i64>,
}

pub async fn handle_request(ctx: &mut ReplContext, request: JsonRequest) -> JsonResponse {
    let id = request.id.clone();
    match dispatch(ctx, request).await {
        Ok(result) => JsonResponse::success(id, result),
        Err(err) => JsonResponse::failure(id, format!("{err:#}")),
    }
}

async fn dispatch(ctx: &mut ReplContext, request: JsonRequest) -> Result<Value> {
    let op = request.op.to_ascii_lowercase().replace('-', "_");
    match op.as_str() {
        "help" => Ok(help_json()),
        "quit" | "exit" | "stop" => Ok(json!({ "disconnected": true })),
        "continue" | "run" => ctx.navigate(NavigationType::Continue).await,
        "step_over" | "next" => ctx.navigate(NavigationType::StepOver).await,
        "step_in" | "step" => ctx.navigate(NavigationType::StepIn).await,
        "step_out" | "finish" => ctx.navigate(NavigationType::StepOut).await,
        "step_back" => ctx.navigate(NavigationType::StepBack).await,
        "reverse_continue" => ctx.navigate(NavigationType::ReverseContinue).await,
        "pause" => ctx.navigate(NavigationType::Pause).await,
        "threads" => ctx.threads().await,
        "stack" | "backtrace" => ctx.stack().await,
        "status" => ctx.status().await,
        "sync" => ctx.sync().await,
        "show" | "src" | "code" => ctx.show(request.context_lines, false).await,
        "capabilities" | "caps" => Ok(serde_json::to_value(ctx.capabilities())?),
        "config" => Ok(ctx.session_config()),
        "read_memory" => {
            let memory_reference = request
                .memory_reference
                .context("read_memory requires memory_reference")?;
            ctx.read_memory(
                &memory_reference,
                request.count,
                request.offset,
            )
            .await
        }
        "write_memory" => {
            let memory_reference = request
                .memory_reference
                .context("write_memory requires memory_reference")?;
            let data = request.data.context("write_memory requires data")?;
            ctx.write_memory(&memory_reference, &data, request.offset).await
        }
        "thread_snapshot" => ctx
            .thread_snapshot(
                request.include_stacks.unwrap_or(true),
                request.stack_depth,
                request.max_threads,
            )
            .await,
        "scopes" => ctx.scopes().await,
        "locals" | "variables" => {
            if let Some(reference) = request.variables_reference {
                ctx.variables(reference).await
            } else {
                ctx.locals().await
            }
        }
        "evaluate" | "print" => {
            let expression = request
                .expression
                .context("evaluate requires expression")?;
            ctx.evaluate(expression).await
        }
        "thread" => {
            let thread_id = request
                .thread_id
                .context("thread requires thread_id")?;
            ctx.set_thread(thread_id).await
        }
        "frame" => {
            let frame_id = request.frame_id.context("frame requires frame_id")?;
            ctx.set_frame(frame_id).await
        }
        "breakpoint" | "break" | "trace" => {
            let path = request.path.context("breakpoint requires path")?;
            let line = request.line.context("breakpoint requires line")?;
            let action = parse_breakpoint_action(request.action.as_deref())?;
            ctx.breakpoint(BreakpointRequest {
                path,
                line,
                action,
                options: BreakpointOptions {
                    condition: request.condition,
                    hit_condition: request.hit_condition,
                    log_message: request.log_message,
                },
            })
            .await
        }
        "clear" => {
            let path = request.path.context("clear requires path")?;
            ctx.clear_breakpoints(path, request.line).await
        }
        "skip_list" | "skip" => Ok(ctx.skip_list()),
        "skip_add" => {
            let pattern = request
                .pattern
                .context("skip_add requires pattern")?;
            Ok(ctx.skip_add(pattern))
        }
        "skip_clear" => Ok(ctx.skip_clear(request.pattern)),
        "breakpoints" | "info_break" => Ok(ctx.list_breakpoints()),
        "set_variable" | "set" => {
            let name = request.name.context("set_variable requires name")?;
            let value = request.value.context("set_variable requires value")?;
            ctx.set_variable(name, value, request.variables_reference).await
        }
        "exception_breakpoint" | "catch" => {
            let action = parse_exception_breakpoint_action(request.action.as_deref())?;
            ctx.exception_breakpoint(request.filter, request.condition, action)
                .await
        }
        "exception_filters" | "info_catch" => Ok(ctx.exception_filters()),
        "watch_add" => {
            let expression = request
                .expression
                .context("watch_add requires expression")?;
            ctx.watch_add(expression).await
        }
        "watch_remove" | "unwatch" => {
            let expression = request
                .expression
                .context("watch_remove requires expression")?;
            ctx.watch_remove(&expression).await
        }
        "watch_list" | "watches" => Ok(ctx.watch_list()),
        "smart_step" => {
            let enabled = request
                .value
                .as_deref()
                .map(parse_bool)
                .transpose()?
                .context("smart_step requires value (true/false)")?;
            Ok(ctx.set_smart_step(enabled))
        }
        "suppress_entry_stop" => {
            let enabled = request
                .value
                .as_deref()
                .map(parse_bool)
                .transpose()?
                .context("suppress_entry_stop requires value (true/false)")?;
            Ok(ctx.set_suppress_entry_stop(enabled))
        }
        "function_breakpoint" | "break_function" => {
            let action = parse_function_breakpoint_action(request.action.as_deref())?;
            ctx.function_breakpoint(request.name, action).await
        }
        "goto" => {
            let path = request.path.context("goto requires path")?;
            let line = request.line.context("goto requires line")?;
            ctx.goto_line(path, line).await
        }
        "goto_targets" => {
            let path = request.path.context("goto_targets requires path")?;
            let line = request.line.context("goto_targets requires line")?;
            ctx.goto_targets(path, line).await
        }
        "completions" | "complete" => {
            let text = request
                .expression
                .or(request.text)
                .context("completions requires expression or text")?;
            let column = request.column.unwrap_or(text.len() as i64);
            ctx.completions(text, column).await
        }
        "data_watch_add" => {
            let expression = request
                .expression
                .context("data_watch_add requires expression")?;
            ctx.data_watch_add(expression).await
        }
        "data_watch_remove" => {
            let expression = request
                .expression
                .context("data_watch_remove requires expression")?;
            ctx.data_watch_remove(&expression).await
        }
        "data_watch_list" | "data_watches" => Ok(ctx.data_watch_list()),
        "restart_frame" | "restart" => ctx.restart_frame(request.frame_id).await,
        "disassemble" | "disasm" => {
            let memory_reference = request
                .memory_reference
                .context("disassemble requires memory_reference")?;
            let offset = request.offset.unwrap_or(0);
            let count = request.count.unwrap_or(1);
            ctx.disassemble(&memory_reference, offset, count).await
        }
        "instruction_breakpoint" => {
            let memory_reference = request
                .memory_reference
                .context("instruction_breakpoint requires memory_reference")?;
            let offset = request.offset.unwrap_or(0);
            let enabled = request
                .value
                .as_deref()
                .map(parse_bool)
                .transpose()?
                .unwrap_or(true);
            ctx.instruction_breakpoint(memory_reference, offset, enabled)
                .await
        }
        "dap" | "dap_request" => {
            let command = request.command.context("dap requires command")?;
            ctx.dap_request(&command, request.arguments).await
        }
        "context" | "state" => Ok(ctx.ready_state()),
        other => anyhow::bail!("unknown op: {other}"),
    }
}

fn parse_exception_breakpoint_action(action: Option<&str>) -> Result<ExceptionBreakpointAction> {
    match action.map(|s| s.to_ascii_lowercase()).as_deref() {
        None | Some("add") | Some("set") | Some("catch") => Ok(ExceptionBreakpointAction::Add),
        Some("remove") | Some("delete") => Ok(ExceptionBreakpointAction::Remove),
        Some("clear") => Ok(ExceptionBreakpointAction::Clear),
        Some(other) => anyhow::bail!("unknown exception breakpoint action: {other}"),
    }
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => anyhow::bail!("expected boolean value, got '{other}'"),
    }
}

fn parse_function_breakpoint_action(action: Option<&str>) -> Result<FunctionBreakpointAction> {
    match action.map(|s| s.to_ascii_lowercase()).as_deref() {
        None | Some("add") | Some("set") => Ok(FunctionBreakpointAction::Add),
        Some("remove") | Some("delete") => Ok(FunctionBreakpointAction::Remove),
        Some("clear") => Ok(FunctionBreakpointAction::Clear),
        Some(other) => anyhow::bail!("unknown function breakpoint action: {other}"),
    }
}

fn parse_breakpoint_action(action: Option<&str>) -> Result<BreakpointAction> {
    match action.map(|s| s.to_ascii_lowercase()).as_deref() {
        None | Some("add") | Some("set") => Ok(BreakpointAction::Add),
        Some("remove") | Some("delete") => Ok(BreakpointAction::Remove),
        Some("toggle") => Ok(BreakpointAction::Toggle),
        Some("clear") => Ok(BreakpointAction::Remove),
        Some(other) => anyhow::bail!("unknown breakpoint action: {other}"),
    }
}

fn help_json() -> Value {
    json!({
        "ops": [
            { "op": "continue" },
            { "op": "step_over" },
            { "op": "step_in" },
            { "op": "step_out" },
            { "op": "breakpoint", "path": "main.rs", "line": 10, "condition": "x > 5", "hit_condition": "3", "log_message": "here" },
            { "op": "trace", "path": "main.rs", "line": 10, "log_message": "value={x}" },
            { "op": "clear", "path": "main.rs", "line": 10 },
            { "op": "skip_list" },
            { "op": "skip_add", "pattern": "/vendor/" },
            { "op": "skip_clear" },
            { "op": "sync" },
            { "op": "show" },
            { "op": "show", "context_lines": 3 },
            { "op": "capabilities" },
            { "op": "config" },
            { "op": "read_memory", "memory_reference": "0x1000", "count": 16 },
            { "op": "write_memory", "memory_reference": "0x1000", "data": "48656C6C6F" },
            { "op": "thread_snapshot", "include_stacks": true, "stack_depth": 5 },
            { "op": "thread", "thread_id": 1 },
            { "op": "frame", "frame_id": 1 },
            { "op": "threads" },
            { "op": "stack" },
            { "op": "status" },
            { "op": "scopes" },
            { "op": "locals" },
            { "op": "variables", "variables_reference": 1 },
            { "op": "evaluate", "expression": "x + 1" },
            { "op": "set_variable", "name": "x", "value": "1" },
            { "op": "breakpoints" },
            { "op": "catch", "filter": "uncaught", "action": "add" },
            { "op": "exception_filters" },
            { "op": "watch_add", "expression": "x" },
            { "op": "watch_list" },
            { "op": "watch_remove", "expression": "x" },
            { "op": "smart_step", "value": "true" },
            { "op": "suppress_entry_stop", "value": "true" },
            { "op": "function_breakpoint", "name": "accumulate", "action": "add" },
            { "op": "goto", "path": "main.py", "line": 50 },
            { "op": "goto_targets", "path": "main.py", "line": 50 },
            { "op": "completions", "expression": "acc" },
            { "op": "data_watch_add", "expression": "x" },
            { "op": "data_watch_list" },
            { "op": "restart_frame", "frame_id": 1 },
            { "op": "disassemble", "memory_reference": "0x1000", "offset": 0, "count": 4 },
            { "op": "instruction_breakpoint", "memory_reference": "0x1000", "offset": 0, "value": "true" },
            { "op": "dap", "command": "threads", "arguments": {} },
            { "op": "context" },
            { "op": "quit" }
        ],
        "protocol": "one JSON object per line on stdin; one JSON response per line on stdout"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_breakpoint_action() {
        assert_eq!(
            parse_breakpoint_action(Some("toggle")).unwrap(),
            BreakpointAction::Toggle
        );
        assert_eq!(parse_breakpoint_action(None).unwrap(), BreakpointAction::Add);
    }

    #[test]
    fn json_request_deserializes_extended_breakpoint() {
        let request: JsonRequest = serde_json::from_str(
            r#"{"id":1,"op":"breakpoint","path":"main.rs","line":10,"hit_condition":"3","log_message":"here"}"#,
        )
        .unwrap();
        assert_eq!(request.hit_condition.as_deref(), Some("3"));
        assert_eq!(request.log_message.as_deref(), Some("here"));
    }

    #[test]
    fn parses_exception_breakpoint_action() {
        assert_eq!(
            parse_exception_breakpoint_action(Some("clear")).unwrap(),
            ExceptionBreakpointAction::Clear
        );
        assert_eq!(
            parse_exception_breakpoint_action(Some("catch")).unwrap(),
            ExceptionBreakpointAction::Add
        );
    }

    #[tokio::test]
    async fn dispatch_help_and_unknown_op() {
        let adapter_dir = std::env::var("CARGO_BIN_EXE_fake-dap-adapter")
            .ok()
            .and_then(|path| {
                std::path::PathBuf::from(path)
                    .parent()
                    .map(std::path::Path::to_path_buf)
            })
            .unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
                    .join("../target/debug")
            });
        let current = std::env::var("PATH").unwrap_or_default();
        unsafe {
            std::env::set_var("PATH", format!("{}:{}", adapter_dir.display(), current));
        }

        let options = crate::repl::ReplOptions {
            program: Some("main.py".into()),
            adapter: Some("fake".into()),
            target: None,
            ndjson: true,
            globals: crate::commands::GlobalOpts {
                json: false,
                control_port: None,
                scope: None,
            },
        };
        let mut ctx = ReplContext::connect(&options, None).await.expect("connect");

        let help = handle_request(
            &mut ctx,
            JsonRequest {
                id: Some(json!(1)),
                op: "help".into(),
                ..Default::default()
            },
        )
        .await;
        assert!(help.ok);

        let unknown = handle_request(
            &mut ctx,
            JsonRequest {
                id: Some(json!(2)),
                op: "not-real".into(),
                ..Default::default()
            },
        )
        .await;
        assert!(!unknown.ok);

        let sync = handle_request(
            &mut ctx,
            JsonRequest {
                id: Some(json!(3)),
                op: "sync".into(),
                ..Default::default()
            },
        )
        .await;
        assert!(sync.ok);

        let thread = handle_request(
            &mut ctx,
            JsonRequest {
                id: Some(json!(4)),
                op: "thread".into(),
                thread_id: Some(1),
                ..Default::default()
            },
        )
        .await;
        assert!(thread.ok);

        let clear = handle_request(
            &mut ctx,
            JsonRequest {
                id: Some(json!(5)),
                op: "clear".into(),
                path: Some("/fake/main.py".into()),
                line: Some(1),
                ..Default::default()
            },
        )
        .await;
        assert!(clear.ok);

        let _ = ctx.shutdown().await;
    }
}
