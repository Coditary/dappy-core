use std::fs::{self, File};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use dap_protocol::{Message, Request, Response};
use serde_json::{Value, json};

use crate::child_session::decline_reverse_request;

/// Handle adapter-originated reverse requests before they reach multiplex clients.
///
/// Returns `Some(response)` when the proxy answered on behalf of a headless client.
/// Child-session spawning is handled asynchronously in the mux bridge when configured.
pub fn handle_reverse_request(message: &Message) -> Option<Message> {
    let request = match message {
        Message::Request(req) => req,
        _ => return None,
    };

    match request.command.as_str() {
        "startDebugging" => Some(decline_reverse_request(request)),
        "runInTerminal" => Some(handle_run_in_terminal(request)),
        _ => None,
    }
}

fn handle_run_in_terminal(request: &Request) -> Message {
    let arguments = request
        .arguments
        .as_ref()
        .cloned()
        .unwrap_or(Value::Null);

    let cmdline = arguments
        .get("args")
        .and_then(Value::as_array)
        .filter(|args| !args.is_empty());

    let Some(cmdline) = cmdline else {
        return fail_reverse_request(request, "runInTerminal missing args");
    };

    let program = cmdline
        .first()
        .and_then(Value::as_str)
        .unwrap_or_default();
    if program.is_empty() {
        return fail_reverse_request(request, "runInTerminal args[0] must be a command");
    }

    let mut command = Command::new(program);
    for arg in cmdline.iter().skip(1) {
        if let Some(value) = arg.as_str() {
            command.arg(value);
        }
    }

    if let Some(cwd) = arguments.get("cwd").and_then(Value::as_str) {
        command.current_dir(cwd);
    }

    if let Some(env) = arguments.get("env").and_then(Value::as_object) {
        for (key, value) in env {
            if let Some(value) = value.as_str() {
                command.env(key, value);
            }
        }
    }

    command.stdin(Stdio::null());

    let log_path = run_in_terminal_log_path(request.seq);
    if let Err(err) = fs::create_dir_all(log_path.parent().unwrap_or(std::path::Path::new("/tmp"))) {
        return fail_reverse_request(
            request,
            &format!("failed to create runInTerminal log directory: {err}"),
        );
    }
    let log_file = match File::create(&log_path) {
        Ok(file) => file,
        Err(err) => {
            return fail_reverse_request(
                request,
                &format!("failed to create runInTerminal log file: {err}"),
            );
        }
    };
    let stderr_file = match log_file.try_clone() {
        Ok(file) => file,
        Err(err) => {
            return fail_reverse_request(
                request,
                &format!("failed to duplicate runInTerminal log file: {err}"),
            );
        }
    };
    command.stdout(Stdio::from(log_file));
    command.stderr(Stdio::from(stderr_file));

    match command.spawn() {
        Ok(child) => succeed_reverse_request(
            request,
            Some(json!({
                "captured": true,
                "logPath": log_path.to_string_lossy(),
                "processId": child.id(),
            })),
        ),
        Err(err) => fail_reverse_request(request, &format!("failed to spawn terminal process: {err}")),
    }
}

fn run_in_terminal_log_path(request_seq: i64) -> PathBuf {
    std::env::temp_dir()
        .join("dap-run-in-terminal")
        .join(format!("{request_seq}.log"))
}

fn succeed_reverse_request(request: &Request, body: Option<Value>) -> Message {
    Message::Response(Response {
        seq: 0,
        request_seq: request.seq,
        success: true,
        command: Some(request.command.clone()),
        message: None,
        body,
    })
}

fn fail_reverse_request(request: &Request, message: &str) -> Message {
    Message::Response(Response {
        seq: 0,
        request_seq: request.seq,
        success: false,
        command: Some(request.command.clone()),
        message: Some(message.to_string()),
        body: None,
    })
}
