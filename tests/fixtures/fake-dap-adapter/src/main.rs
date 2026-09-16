//! Minimal fake DAP adapter for integration tests.
//!
//! Adapted from Meta Dapper's test fixture (MIT). Handles:
//! initialize → launch → initialized → configurationDone → stopped → threads/stackTrace/scopes/variables/evaluate

use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex};

use anyhow::{Context, Result};
use base64::Engine;
use clap::Parser;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

#[derive(Clone, Copy)]
struct FakeStackFrame {
    id: i64,
    name: &'static str,
    line: i64,
}

/// Deterministic stack positions for demo / integration tests (top frame first).
fn stack_states() -> &'static [Vec<FakeStackFrame>] {
    static STATES: LazyLock<Vec<Vec<FakeStackFrame>>> = LazyLock::new(|| {
        vec![
            vec![
                FakeStackFrame { id: 1, name: "accumulate", line: 50 },
                FakeStackFrame { id: 2, name: "main", line: 76 },
            ],
            vec![
                FakeStackFrame { id: 1, name: "accumulate", line: 51 },
                FakeStackFrame { id: 2, name: "main", line: 76 },
            ],
            vec![
                FakeStackFrame { id: 1, name: "process_item", line: 38 },
                FakeStackFrame { id: 2, name: "accumulate", line: 51 },
                FakeStackFrame { id: 3, name: "main", line: 76 },
            ],
            vec![
                FakeStackFrame { id: 1, name: "normalize", line: 30 },
                FakeStackFrame { id: 2, name: "process_item", line: 38 },
                FakeStackFrame { id: 3, name: "accumulate", line: 51 },
                FakeStackFrame { id: 4, name: "main", line: 76 },
            ],
            vec![
                FakeStackFrame { id: 1, name: "normalize", line: 33 },
                FakeStackFrame { id: 2, name: "process_item", line: 38 },
                FakeStackFrame { id: 3, name: "accumulate", line: 51 },
                FakeStackFrame { id: 4, name: "main", line: 76 },
            ],
            vec![
                FakeStackFrame { id: 1, name: "process_item", line: 40 },
                FakeStackFrame { id: 2, name: "normalize", line: 33 },
                FakeStackFrame { id: 3, name: "accumulate", line: 51 },
                FakeStackFrame { id: 4, name: "main", line: 76 },
            ],
            vec![
                FakeStackFrame { id: 1, name: "process_item", line: 41 },
                FakeStackFrame { id: 2, name: "accumulate", line: 51 },
                FakeStackFrame { id: 3, name: "main", line: 76 },
            ],
            vec![
                FakeStackFrame { id: 1, name: "accumulate", line: 52 },
                FakeStackFrame { id: 2, name: "main", line: 76 },
            ],
            vec![
                FakeStackFrame { id: 1, name: "accumulate", line: 54 },
                FakeStackFrame { id: 2, name: "main", line: 76 },
            ],
            vec![
                FakeStackFrame { id: 1, name: "accumulate", line: 50 },
                FakeStackFrame { id: 2, name: "main", line: 76 },
            ],
        ]
    });
    &*STATES
}

static STEP_INDEX: Mutex<usize> = Mutex::new(0);

fn reset_step_position() {
    if let Ok(mut index) = STEP_INDEX.lock() {
        *index = 0;
    }
}

fn current_step_index() -> usize {
    STEP_INDEX.lock().map(|index| *index).unwrap_or(0)
}

fn set_step_index(index: usize) {
    if let Ok(mut current) = STEP_INDEX.lock() {
        *current = index.min(stack_states().len().saturating_sub(1));
    }
}

fn advance_step(delta: isize) {
    let states = stack_states();
    if states.is_empty() {
        return;
    }
    let next = current_step_index() as isize + delta;
    let clamped = next.clamp(0, states.len() as isize - 1) as usize;
    set_step_index(clamped);
}

fn step_out() {
    let states = stack_states();
    if states.is_empty() {
        return;
    }
    let current = current_step_index();
    let current_depth = states[current].len();
    if current_depth <= 1 {
        return;
    }
    let target_depth = current_depth - 1;
    for index in (current + 1)..states.len() {
        if states[index].len() == target_depth {
            set_step_index(index);
            return;
        }
    }
    set_step_index(states.len() - 1);
}

fn stack_trace_body(source_path: &str) -> Value {
    let states = stack_states();
    let frames = states
        .get(current_step_index())
        .or_else(|| states.first())
        .cloned()
        .unwrap_or_default();
    let stack_frames = frames
        .iter()
        .map(|frame| {
            json!({
                "id": frame.id,
                "name": frame.name,
                "line": frame.line,
                "column": 1,
                "source": { "path": source_path },
            })
        })
        .collect::<Vec<_>>();
    json!({
        "stackFrames": stack_frames,
        "totalFrames": stack_frames.len(),
    })
}

#[derive(Parser, Debug)]
#[command(about = "Fake DAP adapter for integration tests")]
struct Args {
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    supports_step_back: bool,
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    supports_exception_filter_options: bool,
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    supports_set_variable: bool,
    #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
    supports_set_expression: bool,
    #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
    supports_function_breakpoints: bool,
    #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
    supports_goto_request: bool,
    #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
    supports_completions_request: bool,
    /// Emit a `startDebugging` reverse request after configurationDone.
    #[arg(long, default_value_t = false)]
    emit_start_debugging: bool,
}

static SEQ: AtomicI64 = AtomicI64::new(1);
static DATA_WATCH_COUNTER: AtomicU32 = AtomicU32::new(0);
static PROGRAM_PATH: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new("/fake/main.py".to_string()));

fn next_seq() -> i64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

const MAX_BODY: usize = 16 * 1024 * 1024;

async fn read_message<R: AsyncBufReadExt + Unpin>(reader: &mut R) -> Result<Option<Value>> {
    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_length = Some(rest.trim().parse().context("invalid Content-Length")?);
        }
    }

    let len = content_length.context("missing Content-Length")?;
    anyhow::ensure!(len <= MAX_BODY, "Content-Length too large");
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    Ok(Some(serde_json::from_slice(&body)?))
}

async fn write_message<W: AsyncWrite + Unpin>(writer: &mut W, value: Value) -> Result<()> {
    let body = serde_json::to_vec(&value)?;
    writer
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

async fn send_response<W: AsyncWrite + Unpin>(
    writer: &mut W,
    request_seq: i64,
    command: &str,
    body: Option<Value>,
) -> Result<()> {
    let mut response = json!({
        "type": "response",
        "seq": next_seq(),
        "requestSeq": request_seq,
        "success": true,
        "command": command,
    });
    if let Some(body) = body {
        response["body"] = body;
    }
    write_message(writer, response).await
}

async fn send_event<W: AsyncWrite + Unpin>(
    writer: &mut W,
    event: &str,
    body: Option<Value>,
) -> Result<()> {
    let mut payload = json!({
        "type": "event",
        "seq": next_seq(),
        "event": event,
    });
    if let Some(body) = body {
        payload["body"] = body;
    }
    write_message(writer, payload).await
}

async fn send_reverse_request<W: AsyncWrite + Unpin>(
    writer: &mut W,
    command: &str,
    arguments: Value,
) -> Result<()> {
    let request = json!({
        "type": "request",
        "seq": next_seq(),
        "command": command,
        "arguments": arguments,
    });
    write_message(writer, request).await
}

/// Deterministic evaluate results for integration tests (conditions, logpoints).
fn evaluate_for_tests(expression: &str) -> String {
    let trimmed = expression.trim();
    match trimmed {
        "true" | "1" => "true".into(),
        "false" | "0" => "false".into(),
        "data_watch_marker" => DATA_WATCH_COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .to_string(),
        expr if expr.contains("x > 0") => "true".into(),
        expr if expr.contains("x < 0") => "false".into(),
        other => format!("eval({other})"),
    }
}

fn stopped_body(reason: &str) -> Value {
    json!({
        "reason": reason,
        "threadId": 1,
        "allThreadsStopped": true,
    })
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);

    while let Some(message) = read_message(&mut reader).await? {
        if message["type"].as_str() == Some("response") {
            if message["success"].as_bool() != Some(true) {
                eprintln!(
                    "fake-dap-adapter got a failed reverse-request ack: {}",
                    message
                );
                std::process::exit(2);
            }
            continue;
        }
        if message["type"].as_str() != Some("request") {
            continue;
        }
        let request_seq = message["seq"].as_i64().context("missing seq")?;
        let command = message["command"]
            .as_str()
            .context("missing command")?
            .to_string();

        match command.as_str() {
            "initialize" => {
                send_response(
                    &mut stdout,
                    request_seq,
                    "initialize",
                    Some(json!({
                        "supportsConfigurationDoneRequest": true,
                        "supportsStepBack": args.supports_step_back,
                        "supportsReadMemoryRequest": true,
                        "supportsWriteMemoryRequest": true,
                        "supportsSetVariable": args.supports_set_variable,
                        "supportsSetExpression": args.supports_set_expression,
                        "supportsFunctionBreakpoints": args.supports_function_breakpoints,
                        "supportsGotoTargetsRequest": args.supports_goto_request,
                        "supportsGotoRequest": args.supports_goto_request,
                        "supportsCompletionsRequest": args.supports_completions_request,
                        "supportsBreakpointLocationsRequest": true,
                        "supportsDisassembleRequest": true,
                        "supportsExceptionFilterOptions": args.supports_exception_filter_options,
                        "exceptionBreakpointFilters": [
                            { "filter": "uncaught", "label": "Uncaught Exceptions", "default": true },
                            { "filter": "raised", "label": "Raised Exceptions", "default": false },
                        ],
                    })),
                )
                .await?;
            }
            "launch" | "attach" => {
                if let Some(program) = message["arguments"]["program"].as_str() {
                    if let Ok(mut path) = PROGRAM_PATH.lock() {
                        *path = program.to_string();
                    }
                }
                send_response(&mut stdout, request_seq, &command, None).await?;
                send_event(&mut stdout, "initialized", None).await?;
            }
            "configurationDone" => {
                send_response(&mut stdout, request_seq, "configurationDone", None).await?;
                reset_step_position();
                send_event(&mut stdout, "stopped", Some(stopped_body("entry"))).await?;
                if args.emit_start_debugging {
                    send_reverse_request(
                        &mut stdout,
                        "startDebugging",
                        json!({ "request": "launch", "configuration": {} }),
                    )
                    .await?;
                }
            }
            "threads" => {
                send_response(
                    &mut stdout,
                    request_seq,
                    "threads",
                    Some(json!({ "threads": [{ "id": 1, "name": "main" }] })),
                )
                .await?;
            }
            "setBreakpoints" => {
                send_response(&mut stdout, request_seq, "setBreakpoints", None).await?;
            }
            "setFunctionBreakpoints" => {
                send_response(&mut stdout, request_seq, "setFunctionBreakpoints", None).await?;
            }
            "breakpointLocations" => {
                let line = message["arguments"]["line"].as_i64().unwrap_or(1);
                send_response(
                    &mut stdout,
                    request_seq,
                    "breakpointLocations",
                    Some(json!({
                        "breakpoints": [{ "line": line, "column": 1 }],
                    })),
                )
                .await?;
            }
            "setExpression" => {
                let value = message["arguments"]["value"]
                    .as_str()
                    .unwrap_or("?");
                send_response(
                    &mut stdout,
                    request_seq,
                    "setExpression",
                    Some(json!({
                        "value": value,
                        "type": "int",
                        "variablesReference": 0,
                    })),
                )
                .await?;
            }
            "gotoTargets" => {
                let line = message["arguments"]["line"].as_i64().unwrap_or(1);
                send_response(
                    &mut stdout,
                    request_seq,
                    "gotoTargets",
                    Some(json!({
                        "targets": [{ "id": 1, "label": format!("line {}", line), "line": line }],
                    })),
                )
                .await?;
            }
            "goto" => {
                send_response(&mut stdout, request_seq, "goto", None).await?;
                send_event(
                    &mut stdout,
                    "stopped",
                    Some(json!({ "reason": "goto", "threadId": 1 })),
                )
                .await?;
            }
            "completions" => {
                let text = message["arguments"]["text"].as_str().unwrap_or("");
                send_response(
                    &mut stdout,
                    request_seq,
                    "completions",
                    Some(json!({
                        "targets": [{ "label": format!("{text}_complete"), "start": 0, "length": text.len() }],
                    })),
                )
                .await?;
            }
            "setExceptionBreakpoints" => {
                let filters = message["arguments"]["filters"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let breakpoints = filters
                    .iter()
                    .filter_map(|filter| filter.as_str())
                    .map(|filter| json!({ "verified": true, "filter": filter }))
                    .collect::<Vec<_>>();
                send_response(
                    &mut stdout,
                    request_seq,
                    "setExceptionBreakpoints",
                    Some(json!({ "breakpoints": breakpoints })),
                )
                .await?;
            }
            "setVariable" => {
                let name = message["arguments"]["name"]
                    .as_str()
                    .unwrap_or("?");
                let value = message["arguments"]["value"]
                    .as_str()
                    .unwrap_or("?");
                send_response(
                    &mut stdout,
                    request_seq,
                    "setVariable",
                    Some(json!({
                        "value": value,
                        "type": "int",
                        "variablesReference": 0,
                        "name": name,
                    })),
                )
                .await?;
            }
            "stackTrace" => {
                let source_path = PROGRAM_PATH
                    .lock()
                    .map(|path| path.clone())
                    .unwrap_or_else(|_| "/fake/main.py".into());
                send_response(
                    &mut stdout,
                    request_seq,
                    "stackTrace",
                    Some(stack_trace_body(&source_path)),
                )
                .await?;
            }
            "scopes" => {
                send_response(
                    &mut stdout,
                    request_seq,
                    "scopes",
                    Some(json!({
                        "scopes": [{
                            "name": "Locals",
                            "variablesReference": 1,
                            "expensive": false,
                        }],
                    })),
                )
                .await?;
            }
            "variables" => {
                send_response(
                    &mut stdout,
                    request_seq,
                    "variables",
                    Some(json!({
                        "variables": [{
                            "name": "x",
                            "value": "42",
                            "type": "int",
                            "variablesReference": 0,
                        }],
                    })),
                )
                .await?;
            }
            "evaluate" => {
                let expression = message["arguments"]["expression"]
                    .as_str()
                    .unwrap_or("?");
                let result = evaluate_for_tests(expression);
                send_response(
                    &mut stdout,
                    request_seq,
                    "evaluate",
                    Some(json!({
                        "result": result,
                        "variablesReference": 0,
                    })),
                )
                .await?;
            }
            "disassemble" => {
                let memory_reference = message["arguments"]["memoryReference"]
                    .as_str()
                    .unwrap_or("0x0");
                let offset = message["arguments"]["offset"].as_i64().unwrap_or(0);
                let source_path = PROGRAM_PATH
                    .lock()
                    .map(|path| path.clone())
                    .unwrap_or_else(|_| "/fake/main.py".to_string());
                let line = stack_states()
                    .get(current_step_index())
                    .and_then(|frames| frames.first())
                    .map(|frame| frame.line)
                    .unwrap_or(50);
                send_response(
                    &mut stdout,
                    request_seq,
                    "disassemble",
                    Some(json!({
                        "instructions": [{
                            "address": format!("{memory_reference}+{offset}"),
                            "instructionBytes": "90",
                            "instruction": "nop",
                            "line": line,
                            "column": 1,
                            "location": { "path": source_path },
                        }],
                    })),
                )
                .await?;
            }
            "readMemory" => {
                let memory_reference = message["arguments"]["memoryReference"]
                    .as_str()
                    .unwrap_or("0x0");
                let count = message["arguments"]["count"].as_i64().unwrap_or(16);
                let payload = b"Hello, DAP!".repeat((count as usize / 12).max(1));
                let data = payload[..count.min(payload.len() as i64) as usize].to_vec();
                send_response(
                    &mut stdout,
                    request_seq,
                    "readMemory",
                    Some(json!({
                        "address": memory_reference,
                        "data": base64::engine::general_purpose::STANDARD.encode(data),
                    })),
                )
                .await?;
            }
            "writeMemory" => {
                let memory_reference = message["arguments"]["memoryReference"]
                    .as_str()
                    .unwrap_or("0x0");
                let data = message["arguments"]["data"].as_str().unwrap_or("");
                let written = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map(|bytes| bytes.len() as i64)
                    .unwrap_or(0);
                send_response(
                    &mut stdout,
                    request_seq,
                    "writeMemory",
                    Some(json!({
                        "address": memory_reference,
                        "bytesWritten": written,
                    })),
                )
                .await?;
            }
            "continue" | "next" | "stepIn" | "stepBack" | "reverseContinue" => {
                send_response(&mut stdout, request_seq, &command, None).await?;
                if command == "stepBack" || command == "reverseContinue" {
                    advance_step(-1);
                } else {
                    advance_step(1);
                }
                send_event(
                    &mut stdout,
                    "stopped",
                    Some(json!({ "reason": "step", "threadId": 1 })),
                )
                .await?;
            }
            "stepOut" => {
                send_response(&mut stdout, request_seq, "stepOut", None).await?;
                step_out();
                send_event(
                    &mut stdout,
                    "stopped",
                    Some(json!({ "reason": "step", "threadId": 1 })),
                )
                .await?;
            }
            "pause" => {
                send_response(&mut stdout, request_seq, "pause", None).await?;
                send_event(
                    &mut stdout,
                    "stopped",
                    Some(json!({ "reason": "pause", "threadId": 1 })),
                )
                .await?;
            }
            "disconnect" | "terminate" => {
                send_response(&mut stdout, request_seq, &command, None).await?;
                break;
            }
            other => {
                write_message(
                    &mut stdout,
                    json!({
                        "type": "response",
                        "seq": next_seq(),
                        "requestSeq": request_seq,
                        "success": false,
                        "command": other,
                        "message": format!("unknown command: {}", other),
                    }),
                )
                .await?;
            }
        }
    }

    Ok(())
}
