mod breakpoint;
mod cancel;
mod commands;
pub mod context;
mod help;
mod input;
pub mod json_proto;
mod session;

use std::io::{self, Write};
use std::sync::Arc;

use anyhow::{Context, Result};
use dap_core::{
    NavigationType, StackTraceOptions, TerminalStyle, format_breakpoints_table,
    format_evaluate, format_evaluate_pretty, format_exception_breakpoints,
    format_exception_filters, format_navigate_status, format_scopes, format_stack_trace_with_options,
    format_status, format_threads, format_variables_table,
};
use serde_json::Value;

use breakpoint::BreakpointRequest;
use cancel::{SharedStartupKill, StartupCancel};
use commands::{ReplCommand, parse_line};
use context::{
    BreakpointAction, ExceptionBreakpointAction, FunctionBreakpointAction, JsonResponse, ReplContext,
};
use help::{help_text, help_topic};
use input::ReplInput;
use json_proto::{JsonRequest, handle_request, ready_event};

#[derive(Clone)]
pub struct ReplOptions {
    pub program: Option<String>,
    pub adapter: Option<String>,
    /// RSP target id from `target.yaml` (e.g. gdbserver, qemu-x86-kernel).
    pub target: Option<String>,
    pub globals: crate::commands::GlobalOpts,
    /// NDJSON mode: one JSON request per stdin line, one JSON response per stdout line.
    pub ndjson: bool,
}

#[derive(Debug, Clone, Copy)]
struct ReplDisplay {
    colors_enabled: bool,
}

impl ReplDisplay {
    fn new() -> Self {
        Self { colors_enabled: false }
    }

    fn style(&self) -> TerminalStyle {
        TerminalStyle {
            enabled: self.colors_enabled,
        }
    }

    fn set_colors(&mut self, enable: Option<bool>) {
        let next = match enable {
            Some(value) => value,
            None => !self.colors_enabled,
        };
        self.colors_enabled = next;
        println!(
            "Syntax colors {}",
            if self.colors_enabled { "on" } else { "off" }
        );
    }
}

pub async fn run(options: ReplOptions) -> Result<()> {
    if options.ndjson {
        run_ndjson(options).await
    } else {
        run_plain(options).await
    }
}

async fn run_plain(options: ReplOptions) -> Result<()> {
    if options.program.is_some() {
        eprintln!("Starting debug session… (Ctrl+C to cancel)");
        eprintln!("  1/3 Spawning debug adapter");
    }

    let Some(mut repl) = connect_cancellable(&options).await? else {
        return Ok(());
    };

    if let Some(program) = &options.program {
        println!("Debugging {program} — type `help` or `quit`.");
    } else {
        println!("Attached — type `help` or `quit`.");
    }

    let mut display = ReplDisplay::new();
    let mut input = ReplInput::new()?;

    loop {
        let prompt = repl.repl_prompt();
        let Some(line) = read_interactive_line(&mut input, &prompt).await? else {
            eprintln!("\nInterrupted.");
            break;
        };

        match parse_line(&line) {
            ReplCommand::Empty => continue,
            ReplCommand::Quit => break,
            ReplCommand::Help { topic } => print_help(topic.as_deref()),
            ReplCommand::Version => {
                println!("dap-cli {}", env!("CARGO_PKG_VERSION"));
            }
            ReplCommand::Colors { enable } => display.set_colors(enable),
            command => {
                if let Err(err) = execute_plain(&mut repl, command, &display).await {
                    eprintln!("error: {err:#}");
                }
            }
        }
    }

    repl.shutdown().await?;
    println!("Session ended.");
    Ok(())
}

async fn connect_cancellable(options: &ReplOptions) -> Result<Option<ReplContext>> {
    let cancel = StartupCancel::register().context("register startup cancel handlers")?;
    let kill = Arc::new(SharedStartupKill::new());
    let options = options.clone();
    let kill_for_connect = kill.clone();
    let handle = tokio::spawn(async move {
        ReplContext::connect(&options, Some(kill_for_connect)).await
    });
    let abort = handle.abort_handle();

    tokio::select! {
        result = handle => match result {
            Ok(Ok(ctx)) => Ok(Some(ctx)),
            Ok(Err(err)) => Err(err),
            Err(join_err) => Err(join_err.into()),
        },
        _ = cancel.wait() => {
            kill.kill_all();
            abort.abort();
            eprintln!("\nStartup cancelled (Ctrl+C).");
            Ok(None)
        }
    }
}

async fn read_interactive_line(input: &mut ReplInput, prompt: &str) -> Result<Option<String>> {
    let prompt = prompt.to_string();
    tokio::task::block_in_place(|| input.read_line_with_prompt(&prompt))
}

async fn run_ndjson(options: ReplOptions) -> Result<()> {
    if options.program.is_some() {
        eprintln!("Starting debug session… (Ctrl+C to cancel)");
    }
    let Some(mut repl) = connect_cancellable(&options).await? else {
        return Ok(());
    };
    let mut stdout = io::stdout();
    let stdin = io::stdin();
    let mut line = String::new();

    emit_ndjson(
        &mut stdout,
        JsonResponse::success(None, ready_event(&repl)),
    )?;

    loop {
        line.clear();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: JsonRequest = match serde_json::from_str(trimmed) {
            Ok(request) => request,
            Err(err) => {
                emit_ndjson(
                    &mut stdout,
                    JsonResponse::failure(None, format!("invalid JSON request: {err}")),
                )?;
                continue;
            }
        };

        if request.op.eq_ignore_ascii_case("quit")
            || request.op.eq_ignore_ascii_case("exit")
            || request.op.eq_ignore_ascii_case("stop")
        {
            let response = handle_request(&mut repl, request).await;
            emit_ndjson(&mut stdout, response)?;
            break;
        }

        let response = handle_request(&mut repl, request).await;
        emit_ndjson(&mut stdout, response)?;
    }

    repl.shutdown().await?;
    Ok(())
}

fn emit_ndjson(stdout: &mut io::Stdout, response: JsonResponse) -> Result<()> {
    writeln!(stdout, "{}", response.to_line())?;
    stdout.flush()?;
    Ok(())
}

async fn execute_plain(ctx: &mut ReplContext, command: ReplCommand, display: &ReplDisplay) -> Result<()> {
    let style = display.style();
    match command {
        ReplCommand::Continue => {
            let body = ctx.navigate(NavigationType::Continue).await?;
            print_navigate_plain(&body, Some(style));
        }
        ReplCommand::StepOver => {
            let body = ctx.navigate(NavigationType::StepOver).await?;
            print_navigate_plain(&body, Some(style));
        }
        ReplCommand::StepIn => {
            let body = ctx.navigate(NavigationType::StepIn).await?;
            print_navigate_plain(&body, Some(style));
        }
        ReplCommand::StepOut => {
            let body = ctx.navigate(NavigationType::StepOut).await?;
            print_navigate_plain(&body, Some(style));
        }
        ReplCommand::Threads => {
            let body = ctx.threads().await?;
            print!("{}", format_threads(&body));
        }
        ReplCommand::Stack { full } => {
            let body = ctx.stack().await?;
            let options = StackTraceOptions {
                full,
                style: Some(style),
            };
            print!("{}", format_stack_trace_with_options(&body, options));
        }
        ReplCommand::Status => {
            let body = ctx.status().await?;
            if let Ok(summary) = serde_json::from_value::<dap_core::VersionedExecutionState>(body) {
                print!("{}", format_status(&summary));
            }
        }
        ReplCommand::Sync => {
            let body = ctx.sync().await?;
            if let Ok(summary) = serde_json::from_value::<dap_core::VersionedExecutionState>(
                body.get("execution").cloned().unwrap_or(Value::Null),
            ) {
                print!("{}", format_status(&summary));
            }
            if let Some(stack) = body.get("stack") {
                print!(
                    "{}",
                    format_stack_trace_with_options(
                        stack,
                        StackTraceOptions {
                            full: false,
                            style: Some(style),
                        },
                    )
                );
            }
            if let Some(breakpoints) = body.get("breakpoints") {
                print!("{}", format_breakpoints_table(breakpoints, true, Some(style)));
            }
        }
        ReplCommand::InfoBreak => {
            let breakpoints = ctx.list_breakpoints();
            print!("{}", format_breakpoints_table(&breakpoints, true, Some(style)));
        }
        ReplCommand::InfoCatch => {
            let body = ctx.exception_filters();
            if let Some(filters) = body.get("filters").and_then(Value::as_array) {
                print!("{}", format_exception_filters(filters));
            }
            if let Some(installed) = body.get("installed").and_then(Value::as_array) {
                print!("{}", format_exception_breakpoints(installed));
            }
        }
        ReplCommand::Break { path, line, options } => {
            let body = ctx
                .breakpoint(BreakpointRequest {
                    path,
                    line,
                    action: BreakpointAction::Add,
                    options,
                })
                .await?;
            print_breakpoint_result(&body, style);
        }
        ReplCommand::Trace { path, line, message } => {
            let body = ctx
                .breakpoint(BreakpointRequest {
                    path,
                    line,
                    action: BreakpointAction::Add,
                    options: breakpoint::BreakpointOptions {
                        log_message: Some(message),
                        ..Default::default()
                    },
                })
                .await?;
            print_breakpoint_result(&body, style);
        }
        ReplCommand::Clear { path, line } => {
            let cleared_line = line;
            let body = ctx.clear_breakpoints(path, line).await?;
            if let Some(line) = cleared_line {
                println!(
                    "Cleared breakpoint at {}:{}",
                    body["path"].as_str().unwrap_or("?"),
                    line
                );
            } else {
                println!("Cleared all breakpoints in {}", body["path"].as_str().unwrap_or("?"));
            }
        }
        ReplCommand::Eval { expression, pretty } => {
            let body = ctx.evaluate(expression).await?;
            if pretty {
                println!("{}", format_evaluate_pretty(&body, Some(style)));
            } else {
                println!("{}", format_evaluate(&body));
            }
        }
        ReplCommand::Scopes => {
            let body = ctx.scopes().await?;
            print!("{}", format_scopes(&body));
        }
        ReplCommand::Locals => {
            let body = ctx.locals().await?;
            if let Some(vars) = body.get("variables") {
                print!("{}", format_variables_table(vars, Some(style)));
            }
        }
        ReplCommand::Frame(frame_id) => {
            let body = ctx.set_frame(frame_id).await?;
            println!("Current frame: {frame_id}");
            if let Some(stack) = body.get("stack") {
                print!(
                    "{}",
                    format_stack_trace_with_options(
                        stack,
                        StackTraceOptions {
                            full: false,
                            style: Some(style),
                        },
                    )
                );
            }
        }
        ReplCommand::Thread(thread_id) => {
            ctx.set_thread(thread_id).await?;
            println!("Current thread: {thread_id}");
        }
        ReplCommand::Help { .. }
        | ReplCommand::Quit
        | ReplCommand::Empty
        | ReplCommand::Version
        | ReplCommand::Colors { .. } => {}
        ReplCommand::Capabilities => {
            println!("{}", serde_json::to_string_pretty(ctx.capabilities())?);
        }
        ReplCommand::Show { context } => {
            let body = ctx.show(context, display.colors_enabled).await?;
            if let Some(display) = body.get("display").and_then(Value::as_str) {
                print!("{}", display);
            }
        }
        ReplCommand::SkipList => {
            println!("{}", ctx.skip_list());
        }
        ReplCommand::SkipAdd { pattern } => {
            println!("{}", ctx.skip_add(pattern));
        }
        ReplCommand::SkipClear { pattern } => {
            println!("{}", ctx.skip_clear(pattern));
        }
        ReplCommand::SetVariable { name, value } => {
            let body = ctx.set_variable(name, value, None).await?;
            let name = body.get("name").and_then(Value::as_str).unwrap_or("?");
            let value = body.get("value").and_then(Value::as_str).unwrap_or("?");
            let assignment = format!("{name} = {value}");
            if display.colors_enabled {
                println!("{}", style.green(&assignment));
            } else {
                println!("{assignment}");
            }
        }
        ReplCommand::Catch {
            filter,
            condition,
            clear,
        } => {
            let action = if clear {
                if filter.is_some() {
                    ExceptionBreakpointAction::Remove
                } else {
                    ExceptionBreakpointAction::Clear
                }
            } else {
                ExceptionBreakpointAction::Add
            };
            let body = ctx.exception_breakpoint(filter, condition, action).await?;
            if let Some(installed) = body.get("installed").and_then(Value::as_array) {
                print!("{}", format_exception_breakpoints(installed));
            }
        }
        ReplCommand::WatchList => {
            let body = ctx.watch_list();
            if let Some(expressions) = body.get("expressions").and_then(Value::as_array) {
                for expression in expressions {
                    println!("{}", expression.as_str().unwrap_or_default());
                }
            }
        }
        ReplCommand::WatchAdd { expression } => {
            let body = ctx.watch_add(expression).await?;
            if let Some(watches) = body.get("watches").and_then(Value::as_array) {
                for watch in watches {
                    let expr = watch.get("expression").and_then(Value::as_str).unwrap_or("?");
                    let result = watch.get("result").and_then(Value::as_str).unwrap_or("?");
                    println!("{expr} = {result}");
                }
            }
        }
        ReplCommand::WatchRemove { expression } => {
            let _ = ctx.watch_remove(&expression).await?;
            println!("removed watch: {expression}");
        }
        ReplCommand::SmartStep { enable } => {
            let enabled = enable.unwrap_or(true);
            let body = ctx.set_smart_step(enabled);
            println!("smart-step {}", if body["smart_step"].as_bool().unwrap_or(false) { "on" } else { "off" });
        }
        ReplCommand::Goto { path, line } => {
            let body = ctx.goto_line(path, line).await?;
            if let Some(method) = body.get("method").and_then(Value::as_str) {
                println!("goto via {method}");
            }
            print_navigate_plain(&body, Some(style));
        }
        ReplCommand::BreakFunction { name, action } => {
            let name = if action == FunctionBreakpointAction::Clear || name.is_empty() {
                None
            } else {
                Some(name)
            };
            let body = ctx.function_breakpoint(name, action).await?;
            if let Some(installed) = body.get("installed").and_then(Value::as_array) {
                for entry in installed {
                    let name = entry.get("name").and_then(Value::as_str).unwrap_or("?");
                    let line = entry
                        .get("resolved_line")
                        .and_then(Value::as_i64)
                        .map(|line| format!(" @ line {line}"))
                        .unwrap_or_default();
                    println!("function breakpoint: {name}{line}");
                }
            }
        }
        ReplCommand::DataWatchList => {
            let body = ctx.data_watch_list();
            if let Some(watches) = body.get("watches").and_then(Value::as_array) {
                for watch in watches {
                    let expression = watch.get("expression").and_then(Value::as_str).unwrap_or("?");
                    println!("{expression}");
                }
            }
        }
        ReplCommand::DataWatchAdd { expression } => {
            let body = ctx.data_watch_add(expression).await?;
            println!(
                "data watch added (emulated={})",
                body.get("emulated").and_then(Value::as_bool).unwrap_or(false)
            );
        }
        ReplCommand::DataWatchRemove { expression } => {
            let _ = ctx.data_watch_remove(&expression).await?;
            println!("removed data watch: {expression}");
        }
        ReplCommand::RestartFrame { frame_id } => {
            let body = ctx.restart_frame(frame_id).await?;
            let method = body.get("method").and_then(Value::as_str).unwrap_or("restart");
            println!("restart via {method}");
        }
        ReplCommand::Disassemble {
            memory_reference,
            offset,
            count,
        } => {
            let body = ctx.disassemble(&memory_reference, offset, count).await?;
            println!("{}", serde_json::to_string_pretty(&body["body"])?);
        }
        ReplCommand::Unknown(message) => println!("{message}"),
    }
    Ok(())
}

fn print_navigate_plain(body: &Value, style: Option<TerminalStyle>) {
    if let Some(logs) = body.get("logs").and_then(Value::as_array) {
        for log in logs {
            if let Some(message) = log.as_str() {
                println!("LOG: {message}");
            }
        }
    }
    if let Some(nav) = body.get("navigation") {
        let stack = body.get("stack").cloned().unwrap_or(Value::Null);
        println!("{}", format_navigate_status(nav, &stack, style));
    }
}

fn print_breakpoint_result(body: &Value, style: TerminalStyle) {
    let path = body["path"].as_str().unwrap_or("?");
    let line = body["line"].as_i64().unwrap_or(0);
    let location = style.cyan(&format!("{path}:{line}"));
    let mut extras = Vec::new();
    if let Some(condition) = body.get("condition").and_then(Value::as_str) {
        extras.push(format!("if {condition}"));
    }
    if let Some(hit) = body.get("hit_condition").and_then(Value::as_str) {
        extras.push(format!("hit {hit}"));
    }
    if let Some(log) = body.get("log_message").and_then(Value::as_str) {
        extras.push(format!("log {log}"));
    }
    let extra_text = if extras.is_empty() {
        String::new()
    } else {
        format!(" {}", extras.join(" "))
    };
    println!("Breakpoint at {location}{extra_text}");
    if body.get("emulated").and_then(Value::as_bool) == Some(true) {
        println!("{}", style.dim("(client-emulated breakpoint policy)"));
    }
    if let Some(warning) = body.get("warning").and_then(Value::as_str) {
        println!("Warning: {warning}");
    }
}

fn print_help(topic: Option<&str>) {
    if let Some(topic) = topic {
        if let Some(text) = help_topic(topic) {
            println!("{text}");
        } else {
            println!("No help for `{topic}`. Type `help` for a command list.");
        }
        return;
    }
    println!("{}", help_text());
}
