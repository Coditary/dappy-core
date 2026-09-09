use anyhow::{Context, Result};
use clap::Parser;
use dap_core::{ControlClient, DapEngine, LaunchOptions, NavigationType, SourceBreakpointSpec, resolve_control_port};
use dap_protocol::Message;
use instance_manager::SessionStore;
use serde_json::Value;

use crate::commands::GlobalOpts;
use crate::repl::{ReplOptions, run as run_repl};
use crate::output::{
    print_evaluate, print_json, print_navigate, print_scopes, print_stack_trace, print_status,
    print_threads, print_variables,
};

#[derive(Parser)]
pub struct Debug {
    #[command(subcommand)]
    command: DebugCommands,
}

#[derive(clap::Subcommand)]
enum DebugCommands {
    /// Start a debug session (registers instance only; use dap-proxy for live sessions)
    Start {
        /// Program to debug
        #[arg(long)]
        program: String,
        /// Explicit adapter id
        #[arg(long)]
        adapter: Option<String>,
    },
    /// List active debug sessions (alias for `session list`)
    Sessions,
    /// Send an arbitrary DAP request to a running session
    Dap {
        /// DAP command name
        command: String,
        /// JSON arguments object
        #[arg(long)]
        arguments: Option<String>,
    },
    /// List threads in a running session
    Threads,
    /// Fetch stack trace for a thread
    StackTrace { thread_id: i64 },
    /// Set breakpoints in a source file
    SetBreakpoints {
        /// Source file path
        path: String,
        /// Breakpoint line numbers
        #[arg(short = 'b', long = "line", required = true)]
        lines: Vec<i64>,
    },
    /// Show execution status for the active session
    Status,
    /// Resume execution until the next stop
    Continue { thread_id: i64 },
    /// Step over the current line
    StepOver { thread_id: i64 },
    /// Step into the current line
    StepIn { thread_id: i64 },
    /// Step out of the current frame
    StepOut { thread_id: i64 },
    /// Reverse one source line (requires adapter support)
    StepBack { thread_id: i64 },
    /// Reverse-continue execution (requires adapter support)
    ReverseContinue { thread_id: i64 },
    /// Pause a running thread
    Pause { thread_id: i64 },
    /// Evaluate an expression in the debugger
    Evaluate {
        /// Expression to evaluate
        expression: String,
        /// Stack frame id (optional)
        #[arg(long)]
        frame_id: Option<i64>,
    },
    /// List variable scopes for a stack frame
    Scopes {
        /// Stack frame id
        frame_id: i64,
    },
    /// List variables for a variables reference
    Variables {
        /// Variables reference from scopes
        variables_reference: i64,
    },
    /// Disconnect from the debug session
    Stop {
        /// Also terminate the debuggee process
        #[arg(long)]
        terminate: bool,
    },
    /// Interactive gdb-style command loop (plain text by default)
    Repl {
        /// Program/binary to debug (starts a new local session)
        program: Option<String>,
        /// Explicit adapter id (default: auto-route, e.g. rust for Cargo binaries)
        #[arg(long)]
        adapter: Option<String>,
        /// NDJSON mode: one JSON request per stdin line, one JSON response per stdout line
        #[arg(long)]
        ndjson: bool,
    },
    /// Start MCP server on stdio (alias for `dap-agent` binary)
    Mcp,
    /// Attach to a running multiplexed session via control port (legacy)
    Attach {
        /// Control attach port published by dap-proxy
        #[arg(long)]
        port: u16,
        /// DAP command to send after attach (default: threads)
        #[arg(long, default_value = "threads")]
        command: String,
    },
}

impl Debug {
    pub async fn run(self, globals: GlobalOpts) -> Result<()> {
        match self.command {
            DebugCommands::Start { program, adapter } => {
                let engine = DapEngine::with_builtin_plugins()?;
                let session = engine
                    .prepare_session(LaunchOptions {
                        request: "launch".into(),
                        program: Some(program),
                        adapter,
                        extra: serde_json::json!({}),
                    })
                    .await?;
                print_json(
                    &serde_json::to_value(&session).context("serialize session")?,
                    globals.json,
                );
            }
            DebugCommands::Sessions => {
                let store = SessionStore::open(SessionStore::default_dir())?;
                let sessions = store.list_active()?;
                let filtered = filter_by_scope(sessions, &globals.scope);
                print_json(&serde_json::json!(filtered), globals.json);
            }
            DebugCommands::Dap { command, arguments } => {
                let args = parse_arguments(arguments)?;
                let mut client = connect_client(&globals).await?;
                let message = client.dap_request(&command, args).await?;
                print_dap_message(&message, globals.json);
            }
            DebugCommands::Threads => {
                let mut client = connect_client(&globals).await?;
                let body = client.threads().await?;
                print_threads(&body, globals.json);
            }
            DebugCommands::StackTrace { thread_id } => {
                let mut client = connect_client(&globals).await?;
                let body = client.stack_trace(thread_id).await?;
                print_stack_trace(&body, globals.json);
            }
            DebugCommands::SetBreakpoints { path, lines } => {
                let mut client = connect_client(&globals).await?;
                let breakpoints = lines
                    .iter()
                    .map(|line| SourceBreakpointSpec::line(*line))
                    .collect::<Vec<_>>();
                let body = client.set_breakpoints(&path, &breakpoints).await?;
                print_json(&body, globals.json);
            }
            DebugCommands::Status => {
                let mut client = connect_client(&globals).await?;
                let tracker = client.probe_status().await?;
                print_status(&tracker.summary(), globals.json);
            }
            DebugCommands::Continue { thread_id } => {
                run_navigate(&globals, NavigationType::Continue, thread_id).await?;
            }
            DebugCommands::StepOver { thread_id } => {
                run_navigate(&globals, NavigationType::StepOver, thread_id).await?;
            }
            DebugCommands::StepIn { thread_id } => {
                run_navigate(&globals, NavigationType::StepIn, thread_id).await?;
            }
            DebugCommands::StepOut { thread_id } => {
                run_navigate(&globals, NavigationType::StepOut, thread_id).await?;
            }
            DebugCommands::StepBack { thread_id } => {
                run_navigate(&globals, NavigationType::StepBack, thread_id).await?;
            }
            DebugCommands::ReverseContinue { thread_id } => {
                run_navigate(&globals, NavigationType::ReverseContinue, thread_id).await?;
            }
            DebugCommands::Pause { thread_id } => {
                run_navigate(&globals, NavigationType::Pause, thread_id).await?;
            }
            DebugCommands::Evaluate {
                expression,
                frame_id,
            } => {
                let mut client = connect_client(&globals).await?;
                let body = client.evaluate(&expression, frame_id).await?;
                print_evaluate(&body, globals.json);
            }
            DebugCommands::Scopes { frame_id } => {
                let mut client = connect_client(&globals).await?;
                let body = client.scopes(frame_id).await?;
                print_scopes(&body, globals.json);
            }
            DebugCommands::Variables { variables_reference } => {
                let mut client = connect_client(&globals).await?;
                let body = client.variables(variables_reference).await?;
                print_variables(&body, globals.json);
            }
            DebugCommands::Stop { terminate } => {
                let mut client = connect_client(&globals).await?;
                if terminate {
                    client.terminate().await?;
                } else {
                    client.disconnect().await?;
                }
                if globals.json {
                    print_json(
                        &serde_json::json!({
                            "disconnected": true,
                            "terminate_debuggee": terminate,
                        }),
                        globals.json,
                    );
                } else if terminate {
                    println!("Disconnected and terminated debuggee.");
                } else {
                    println!("Disconnected.");
                }
            }
            DebugCommands::Repl { program, adapter, ndjson } => {
                run_repl(ReplOptions {
                    program,
                    adapter,
                    ndjson: ndjson || globals.json,
                    globals,
                })
                .await?;
            }
            DebugCommands::Mcp => {
                run_mcp_agent(&globals)?;
            }
            DebugCommands::Attach { port, command } => {
                let mut client = ControlClient::connect(port).await?;
                let message = client.dap_request(&command, None).await?;
                print_dap_message(&message, globals.json);
            }
        }
        Ok(())
    }
}

async fn run_navigate(
    globals: &GlobalOpts,
    navigation_type: NavigationType,
    thread_id: i64,
) -> Result<()> {
    let mut client = connect_client(globals).await?;
    let result = client.navigate(navigation_type, thread_id).await?;
    print_navigate(&result, globals.json);
    Ok(())
}

fn run_mcp_agent(globals: &GlobalOpts) -> Result<()> {
    let agent = resolve_dap_agent_binary()?;
    let mut cmd = std::process::Command::new(&agent);
    if let Some(port) = globals.control_port {
        cmd.arg("--control-port").arg(port.to_string());
    }
    if let Some(scope) = &globals.scope {
        cmd.arg("--scope").arg(scope);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        anyhow::bail!("failed to exec {}: {err}", agent.display());
    }

    #[cfg(not(unix))]
    {
        let status = cmd.status().context("run dap-agent")?;
        if !status.success() {
            anyhow::bail!("dap-agent exited with {status}");
        }
        Ok(())
    }
}

fn resolve_dap_agent_binary() -> Result<std::path::PathBuf> {
    if let Ok(path) = std::env::var("DAP_AGENT") {
        return Ok(std::path::PathBuf::from(path));
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(std::path::MAIN_SEPARATOR) {
            let candidate = std::path::Path::new(dir).join("dap-agent");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    let exe = std::env::current_exe().context("current_exe")?;
    let debug_dir = exe.parent().context("dap-cli parent")?;
    let candidate = debug_dir.join("dap-agent");
    if candidate.is_file() {
        return Ok(candidate);
    }
    anyhow::bail!(
        "dap-agent not found (set DAP_AGENT or install dap-agent next to dap-cli); build with: go build -o dap-agent ./dap-agent"
    )
}

async fn connect_client(globals: &GlobalOpts) -> Result<ControlClient> {
    let store = SessionStore::open(SessionStore::default_dir())?;
    let port = resolve_control_port(&store, globals.control_port, globals.scope.as_deref())?;
    ControlClient::connect(port).await
}

fn filter_by_scope(
    sessions: Vec<instance_manager::SessionRecord>,
    scope: &Option<String>,
) -> Vec<instance_manager::SessionRecord> {
    if let Some(scope) = scope {
        sessions
            .into_iter()
            .filter(|s| s.scope.as_deref() == Some(scope.as_str()))
            .collect()
    } else {
        sessions
    }
}

fn parse_arguments(arguments: Option<String>) -> Result<Option<Value>> {
    match arguments {
        Some(text) => Ok(Some(
            serde_json::from_str(&text).context("parse --arguments JSON")?,
        )),
        None => Ok(None),
    }
}

fn print_dap_message(message: &Message, json_mode: bool) {
    let value = match message {
        Message::Response(resp) => serde_json::to_value(resp).unwrap_or(Value::Null),
        other => serde_json::to_value(other).unwrap_or(Value::Null),
    };
    print_json(&value, json_mode);
}

#[cfg(test)]
mod tests {
    use super::*;
    use instance_manager::SessionRecord;

    #[test]
    fn filter_by_scope_returns_all_without_filter() {
        let sessions = vec![SessionRecord {
            instance_id: "a".into(),
            pid: 1,
            control_port: 1,
            adapter_id: "fake".into(),
            program: None,
            scope: None,
            parent_id: None,
            started_at_unix: 0,
        }];
        assert_eq!(filter_by_scope(sessions, &None).len(), 1);
    }

    #[test]
    fn parse_arguments_accepts_json() {
        let args = parse_arguments(Some(r#"{"threadId":1}"#.into())).expect("parse");
        assert_eq!(args.unwrap()["threadId"], 1);
        assert!(parse_arguments(None).unwrap().is_none());
    }
}
