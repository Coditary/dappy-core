use std::sync::Arc;

use anyhow::{Context, Result};
use instance_manager::SessionStore;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use std::time::Duration;

use crate::commands::GlobalOpts;
use crate::mcp::McpOptions;
use crate::repl::context::ReplContext;
use crate::repl::json_proto::{JsonRequest, handle_request};

const INSTRUCTIONS: &str = r#"DAP debug MCP server (built into dap-cli).

Recommended workflow for AI agents:
1. debug_sessions — discover running sessions
2. debug_attach / debug_launch — connect at runtime (or use CLI flags on startup)
3. debug_sync or debug_inspect — refresh state after attach or stop
4. debug_set_breakpoints / debug_breakpoint — set breakpoints (with optional conditions)
5. debug_navigate — continue, step_over, step_in, step_out, pause
6. debug_wait_for_stop — block until the next stop (after continue/step)
7. debug_inspect — one-shot status + stack + source + locals
8. debug_evaluate / debug_variables — drill into data
9. debug_terminate or debug_stop — end session when done

Use debug_dap_request only when no dedicated tool exists. JSON results are returned as text."#;

#[derive(Clone)]
pub struct DebugMcpServer {
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
    state: Arc<Mutex<McpState>>,
}

struct McpState {
    repl: Option<ReplContext>,
    repl_options: ReplOptionsSnapshot,
}

#[derive(Clone)]
struct ReplOptionsSnapshot {
    globals: GlobalOpts,
    program: Option<String>,
    adapter: Option<String>,
    target: Option<String>,
}

#[tool_router]
impl DebugMcpServer {
    fn new(options: McpOptions) -> Self {
        Self {
            tool_router: Self::tool_router(),
            state: Arc::new(Mutex::new(McpState {
                repl: None,
                repl_options: ReplOptionsSnapshot {
                    globals: options.globals,
                    program: options.program,
                    adapter: options.adapter,
                    target: options.target,
                },
            })),
        }
    }

    #[tool(description = "List active debug sessions from the session store.")]
    async fn debug_sessions(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<CallToolResult, McpError> {
        let state = self.state.lock().await;
        match list_sessions(&state.repl_options.globals) {
            Ok(sessions) => text_result(&sessions),
            Err(err) => Err(mcp_error(err.to_string())),
        }
    }

    #[tool(description = "Get current execution status from the debug session.")]
    async fn debug_status(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op("status", JsonRequest {
            op: "status".into(),
            ..Default::default()
        })
        .await
    }

    #[tool(description = "List threads in the active debug session.")]
    async fn debug_threads(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op("threads", JsonRequest {
            op: "threads".into(),
            ..Default::default()
        })
        .await
    }

    #[tool(description = "Fetch stack trace for a thread.")]
    async fn debug_stack_trace(
        &self,
        Parameters(params): Parameters<StackTraceParams>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(thread_id) = params.thread_id {
            self.op(
                "thread",
                JsonRequest {
                    op: "thread".into(),
                    thread_id: Some(thread_id),
                    ..Default::default()
                },
            )
            .await?;
        }
        self.op("stack", JsonRequest {
            op: "stack".into(),
            ..Default::default()
        })
        .await
    }

    #[tool(
        description = "Set breakpoints in a source file. Optional condition/hit_condition/log_message apply to every listed line."
    )]
    async fn debug_set_breakpoints(
        &self,
        Parameters(params): Parameters<SetBreakpointsParams>,
    ) -> Result<CallToolResult, McpError> {
        let has_extras = params.condition.is_some()
            || params.hit_condition.is_some()
            || params.log_message.is_some();
        if has_extras {
            let mut state = self.state.lock().await;
            let repl = ensure_repl(&mut state).await?;
            let mut applied = Vec::new();
            for line in params.lines {
                let response = handle_request(
                    repl,
                    JsonRequest {
                        op: "breakpoint".into(),
                        path: Some(params.path.clone()),
                        line: Some(line),
                        action: Some("add".into()),
                        condition: params.condition.clone(),
                        hit_condition: params.hit_condition.clone(),
                        log_message: params.log_message.clone(),
                        ..Default::default()
                    },
                )
                .await;
                if response.ok {
                    applied.push(response.result.unwrap_or(Value::Null));
                } else {
                    return Err(mcp_error(
                        response
                            .error
                            .unwrap_or_else(|| "failed to set breakpoint".into()),
                    ));
                }
            }
            return text_result(&json!({ "applied": applied }));
        }
        let breakpoints = params
            .lines
            .iter()
            .map(|line| json!({ "line": line }))
            .collect::<Vec<_>>();
        self.op(
            "dap",
            JsonRequest {
                op: "dap".into(),
                command: Some("setBreakpoints".into()),
                arguments: Some(json!({
                    "source": { "path": params.path },
                    "breakpoints": breakpoints,
                })),
                ..Default::default()
            },
        )
        .await
    }

    #[tool(
        description = "Set, toggle, or remove a single breakpoint with optional condition, hit count, or log message."
    )]
    async fn debug_breakpoint(
        &self,
        Parameters(params): Parameters<BreakpointParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "breakpoint",
            JsonRequest {
                op: "breakpoint".into(),
                path: Some(params.path),
                line: Some(params.line),
                action: Some(params.action.unwrap_or_else(|| "add".into())),
                condition: params.condition,
                hit_condition: params.hit_condition,
                log_message: params.log_message,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Clear breakpoints in a source file (optional single line).")]
    async fn debug_clear_breakpoints(
        &self,
        Parameters(params): Parameters<ClearBreakpointsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "clear",
            JsonRequest {
                op: "clear".into(),
                path: Some(params.path),
                line: params.line,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(
        description = "List breakpoints tracked by the repl (includes editor breakpoints after attach/sync)."
    )]
    async fn debug_breakpoints(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op("breakpoints", JsonRequest {
            op: "breakpoints".into(),
            ..Default::default()
        })
        .await
    }

    #[tool(
        description = "Refresh stack, execution state, and imported breakpoints from the session."
    )]
    async fn debug_sync(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op("sync", JsonRequest {
            op: "sync".into(),
            ..Default::default()
        })
        .await
    }

    #[tool(description = "Show source code around the current stack frame.")]
    async fn debug_show(
        &self,
        Parameters(params): Parameters<ShowParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "show",
            JsonRequest {
                op: "show".into(),
                context_lines: params.context_lines,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(
        description = "Navigate execution (continue, step_over, step_in, step_out, pause, step_back, reverse_continue)."
    )]
    async fn debug_navigate(
        &self,
        Parameters(params): Parameters<NavigateParams>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(thread_id) = params.thread_id {
            self.op(
                "thread",
                JsonRequest {
                    op: "thread".into(),
                    thread_id: Some(thread_id),
                    ..Default::default()
                },
            )
            .await?;
        }
        self.op(
            &params.navigation_type,
            JsonRequest {
                op: params.navigation_type.clone(),
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Evaluate an expression in the debugger.")]
    async fn debug_evaluate(
        &self,
        Parameters(params): Parameters<EvaluateParams>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(frame_id) = params.frame_id {
            self.op(
                "frame",
                JsonRequest {
                    op: "frame".into(),
                    frame_id: Some(frame_id),
                    ..Default::default()
                },
            )
            .await?;
        }
        self.op(
            "evaluate",
            JsonRequest {
                op: "evaluate".into(),
                expression: Some(params.expression),
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "List variable scopes for a stack frame.")]
    async fn debug_scopes(
        &self,
        Parameters(params): Parameters<ScopesParams>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(frame_id) = params.frame_id {
            self.op(
                "frame",
                JsonRequest {
                    op: "frame".into(),
                    frame_id: Some(frame_id),
                    ..Default::default()
                },
            )
            .await?;
        }
        self.op("scopes", JsonRequest {
            op: "scopes".into(),
            ..Default::default()
        })
        .await
    }

    #[tool(description = "List variables for a variables reference.")]
    async fn debug_variables(
        &self,
        Parameters(params): Parameters<VariablesParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "variables",
            JsonRequest {
                op: "variables".into(),
                variables_reference: Some(params.variables_reference),
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Set a variable in the current frame.")]
    async fn debug_set_variable(
        &self,
        Parameters(params): Parameters<SetVariableParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "set_variable",
            JsonRequest {
                op: "set_variable".into(),
                name: Some(params.name),
                value: Some(params.value),
                variables_reference: params.variables_reference,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Manage exception breakpoints (action: add, remove, clear).")]
    async fn debug_exception_breakpoint(
        &self,
        Parameters(params): Parameters<ExceptionBreakpointParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "exception_breakpoint",
            JsonRequest {
                op: "exception_breakpoint".into(),
                filter: params.filter,
                condition: params.condition,
                action: params.action,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Query adapter capabilities (supportsStepBack, readMemory, etc.).")]
    async fn debug_capabilities(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op("capabilities", JsonRequest {
            op: "capabilities".into(),
            ..Default::default()
        })
        .await
    }

    #[tool(description = "Get session metadata (program, adapter, current context).")]
    async fn debug_config(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op("config", JsonRequest {
            op: "config".into(),
            ..Default::default()
        })
        .await
    }

    #[tool(description = "Read memory from the debugged process at a memory reference.")]
    async fn debug_read_memory(
        &self,
        Parameters(params): Parameters<ReadMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "read_memory",
            JsonRequest {
                op: "read_memory".into(),
                memory_reference: Some(params.memory_reference),
                count: params.count,
                offset: params.offset,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Write hex-encoded bytes to process memory at a memory reference.")]
    async fn debug_write_memory(
        &self,
        Parameters(params): Parameters<WriteMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "write_memory",
            JsonRequest {
                op: "write_memory".into(),
                memory_reference: Some(params.memory_reference),
                data: Some(params.data),
                offset: params.offset,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Snapshot all threads (optionally with stack traces) in one call.")]
    async fn debug_thread_snapshot(
        &self,
        Parameters(params): Parameters<ThreadSnapshotParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "thread_snapshot",
            JsonRequest {
                op: "thread_snapshot".into(),
                include_stacks: params.include_stacks,
                stack_depth: params.stack_depth,
                max_threads: params.max_threads,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Add, remove, or list emulated data watches (action: add, remove, list).")]
    async fn debug_data_watch(
        &self,
        Parameters(params): Parameters<DataWatchParams>,
    ) -> Result<CallToolResult, McpError> {
        let op = match params.action.as_str() {
            "add" => "data_watch_add",
            "remove" => "data_watch_remove",
            "list" | "" => "data_watch_list",
            other => return Err(mcp_error(format!("unknown data watch action: {other}"))),
        };
        self.op(
            op,
            JsonRequest {
                op: op.into(),
                expression: params.expression,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Restart execution from a stack frame (native or emulated via goto).")]
    async fn debug_restart_frame(
        &self,
        Parameters(params): Parameters<RestartFrameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "restart_frame",
            JsonRequest {
                op: "restart_frame".into(),
                frame_id: params.frame_id,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Add, remove, or clear function breakpoints (action: add, remove, clear).")]
    async fn debug_function_breakpoint(
        &self,
        Parameters(params): Parameters<FunctionBreakpointParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "function_breakpoint",
            JsonRequest {
                op: "function_breakpoint".into(),
                action: Some(params.action),
                name: params.name,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Jump execution to a source line (native or emulated).")]
    async fn debug_goto(
        &self,
        Parameters(params): Parameters<GotoParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "goto",
            JsonRequest {
                op: "goto".into(),
                path: Some(params.path),
                line: Some(params.line),
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Add, remove, or list client-side watches (action: add, remove, list).")]
    async fn debug_watch(
        &self,
        Parameters(params): Parameters<WatchParams>,
    ) -> Result<CallToolResult, McpError> {
        let op = match params.action.as_str() {
            "add" => "watch_add",
            "remove" => "watch_remove",
            "list" | "" => "watch_list",
            other => return Err(mcp_error(format!("unknown watch action: {other}"))),
        };
        self.op(
            op,
            JsonRequest {
                op: op.into(),
                expression: params.expression,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Request completions for an expression at a column offset.")]
    async fn debug_completions(
        &self,
        Parameters(params): Parameters<CompletionsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "completions",
            JsonRequest {
                op: "completions".into(),
                expression: Some(params.expression),
                column: params.column,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Disassemble instructions at a memory reference.")]
    async fn debug_disassemble(
        &self,
        Parameters(params): Parameters<DisassembleParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "disassemble",
            JsonRequest {
                op: "disassemble".into(),
                memory_reference: Some(params.memory_reference),
                offset: params.offset,
                count: params.count,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "List local variables for the current stack frame.")]
    async fn debug_locals(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op("locals", JsonRequest {
            op: "locals".into(),
            ..Default::default()
        })
        .await
    }

    #[tool(description = "List available exception filters and installed exception breakpoints.")]
    async fn debug_exception_filters(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op("exception_filters", JsonRequest {
            op: "exception_filters".into(),
            ..Default::default()
        })
        .await
    }

    #[tool(description = "Manage frame skip patterns (action: list, add, clear).")]
    async fn debug_skip(
        &self,
        Parameters(params): Parameters<SkipParams>,
    ) -> Result<CallToolResult, McpError> {
        let op = match params.action.as_str() {
            "add" => "skip_add",
            "clear" => "skip_clear",
            "list" | "" => "skip_list",
            other => return Err(mcp_error(format!("unknown skip action: {other}"))),
        };
        self.op(
            op,
            JsonRequest {
                op: op.into(),
                pattern: params.pattern,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "List valid goto targets for a source line.")]
    async fn debug_goto_targets(
        &self,
        Parameters(params): Parameters<GotoParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "goto_targets",
            JsonRequest {
                op: "goto_targets".into(),
                path: Some(params.path),
                line: Some(params.line),
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Set or clear an instruction breakpoint at a memory reference.")]
    async fn debug_instruction_breakpoint(
        &self,
        Parameters(params): Parameters<InstructionBreakpointParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "instruction_breakpoint",
            JsonRequest {
                op: "instruction_breakpoint".into(),
                memory_reference: Some(params.memory_reference),
                offset: params.offset,
                value: params.enabled.map(|enabled| enabled.to_string()),
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Enable or disable smart-step frame skipping.")]
    async fn debug_smart_step(
        &self,
        Parameters(params): Parameters<BoolToggleParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "smart_step",
            JsonRequest {
                op: "smart_step".into(),
                value: Some(params.enabled.to_string()),
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Enable or disable suppressing the initial entry stop.")]
    async fn debug_suppress_entry_stop(
        &self,
        Parameters(params): Parameters<BoolToggleParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "suppress_entry_stop",
            JsonRequest {
                op: "suppress_entry_stop".into(),
                value: Some(params.enabled.to_string()),
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Select the active thread.")]
    async fn debug_thread(
        &self,
        Parameters(params): Parameters<ThreadParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "thread",
            JsonRequest {
                op: "thread".into(),
                thread_id: Some(params.thread_id),
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Select the active stack frame.")]
    async fn debug_frame(
        &self,
        Parameters(params): Parameters<FrameParams>,
    ) -> Result<CallToolResult, McpError> {
        self.op(
            "frame",
            JsonRequest {
                op: "frame".into(),
                frame_id: Some(params.frame_id),
                ..Default::default()
            },
        )
        .await
    }

    #[tool(
        description = "Wait until the debuggee stops or exits. Use after debug_navigate(continue/step). timeout_ms defaults to 30000."
    )]
    async fn debug_wait_for_stop(
        &self,
        Parameters(params): Parameters<WaitForStopParams>,
    ) -> Result<CallToolResult, McpError> {
        let timeout = Duration::from_millis(params.timeout_ms.unwrap_or(30_000).max(1));
        let mut state = self.state.lock().await;
        let repl = ensure_repl(&mut state).await?;
        let result = repl
            .wait_for_stop(timeout)
            .await
            .map_err(|err| mcp_error(format!("{err:#}")))?;
        text_result(&result)
    }

    #[tool(
        description = "Composite inspection: sync + status + stack + source snippet + locals in one call."
    )]
    async fn debug_inspect(
        &self,
        Parameters(params): Parameters<ShowParams>,
    ) -> Result<CallToolResult, McpError> {
        let context_lines = params.context_lines;
        let mut state = self.state.lock().await;
        let repl = ensure_repl(&mut state).await?;
        let result = repl
            .inspect(context_lines)
            .await
            .map_err(|err| mcp_error(format!("{err:#}")))?;
        text_result(&result)
    }

    #[tool(description = "Launch a new owned debug session at runtime (disconnects any current session).")]
    async fn debug_launch(
        &self,
        Parameters(params): Parameters<LaunchParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        let snapshot = ReplOptionsSnapshot {
            globals: state.repl_options.globals.clone(),
            program: Some(params.program),
            adapter: params.adapter,
            target: params.target,
        };
        let repl = reconnect_repl(&mut state, snapshot).await?;
        text_result(&repl.ready_state())
    }

    #[tool(
        description = "Attach to a running multiplexed session by control port or instance id (disconnects any current session)."
    )]
    async fn debug_attach(
        &self,
        Parameters(params): Parameters<AttachParams>,
    ) -> Result<CallToolResult, McpError> {
        let port = resolve_attach_port(params.control_port, params.instance_id)?;
        let mut state = self.state.lock().await;
        let snapshot = ReplOptionsSnapshot {
            globals: GlobalOpts {
                json: state.repl_options.globals.json,
                control_port: Some(port),
                scope: params.scope.or(state.repl_options.globals.scope.clone()),
            },
            program: None,
            adapter: None,
            target: None,
        };
        let repl = reconnect_repl(&mut state, snapshot).await?;
        text_result(&repl.ready_state())
    }

    #[tool(description = "Terminate the debuggee and end the DAP session.")]
    async fn debug_terminate(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        if let Some(mut repl) = state.repl.take() {
            let result = repl
                .terminate_session()
                .await
                .map_err(|err| mcp_error(format!("{err:#}")))?;
            let _ = repl.shutdown().await;
            state.repl = None;
            return text_result(&result);
        }
        text_result(&json!({ "terminated": false, "reason": "no active session" }))
    }

    #[tool(description = "Send a raw DAP request through the repl (op: dap).")]
    async fn debug_dap_request(
        &self,
        Parameters(params): Parameters<DapRequestParams>,
    ) -> Result<CallToolResult, McpError> {
        let arguments = match params.arguments {
            Some(raw) if !raw.is_null() => Some(raw),
            _ => None,
        };
        self.op(
            "dap",
            JsonRequest {
                op: "dap".into(),
                command: Some(params.command),
                arguments,
                ..Default::default()
            },
        )
        .await
    }

    #[tool(description = "Disconnect from the debug session (repl quit).")]
    async fn debug_stop(
        &self,
        _params: Parameters<EmptyParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        if let Some(repl) = state.repl.take() {
            let mut repl = repl;
            let response = handle_request(
                &mut repl,
                JsonRequest {
                    op: "quit".into(),
                    ..Default::default()
                },
            )
            .await;
            let _ = repl.shutdown().await;
            return json_response(&response);
        }
        text_result(&json!({ "disconnected": true }))
    }

    async fn op(&self, _label: &str, request: JsonRequest) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        let repl = ensure_repl(&mut state).await?;
        let response = handle_request(repl, request).await;
        json_response(&response)
    }

}

#[tool_handler]
impl ServerHandler for DebugMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new("dap-cli", env!("CARGO_PKG_VERSION"));
        info.instructions = Some(INSTRUCTIONS.into());
        info
    }
}

pub async fn run(options: McpOptions) -> Result<()> {
    let service = DebugMcpServer::new(options).serve(stdio()).await?;
    service.waiting().await.context("mcp server stopped")?;
    Ok(())
}

async fn ensure_repl(state: &mut McpState) -> Result<&mut ReplContext, McpError> {
    if state.repl.is_none() {
        let repl = connect_repl(&state.repl_options).await?;
        state.repl = Some(repl);
    }
    Ok(state.repl.as_mut().expect("repl initialized"))
}

async fn reconnect_repl(
    state: &mut McpState,
    snapshot: ReplOptionsSnapshot,
) -> Result<&mut ReplContext, McpError> {
    if let Some(mut repl) = state.repl.take() {
        let _ = repl.shutdown().await;
    }
    state.repl_options = snapshot;
    state.repl = None;
    ensure_repl(state).await
}

async fn connect_repl(snapshot: &ReplOptionsSnapshot) -> Result<ReplContext, McpError> {
    let repl_options = crate::repl::ReplOptions {
        program: snapshot.program.clone(),
        adapter: snapshot.adapter.clone(),
        target: snapshot.target.clone(),
        ndjson: true,
        globals: snapshot.globals.clone(),
    };
    ReplContext::connect(&repl_options, None)
        .await
        .map_err(|err| mcp_error(format!("connect debug session: {err:#}")))
}

fn resolve_attach_port(
    control_port: Option<u16>,
    instance_id: Option<String>,
) -> Result<u16, McpError> {
    if let Some(port) = control_port {
        return Ok(port);
    }
    if let Some(instance_id) = instance_id {
        let store = SessionStore::open(SessionStore::default_dir())
            .map_err(|err| mcp_error(err.to_string()))?;
        let record = store
            .get(&instance_id)
            .map_err(|err| mcp_error(err.to_string()))?
            .ok_or_else(|| mcp_error(format!("session not found: {instance_id}")))?;
        if !record.is_alive() {
            return Err(mcp_error(format!("session {instance_id} is not active")));
        }
        return Ok(record.control_port);
    }
    Err(mcp_error(
        "debug_attach requires control_port or instance_id (or start with --control-port)",
    ))
}

fn list_sessions(globals: &GlobalOpts) -> Result<Value> {
    let store = SessionStore::open(SessionStore::default_dir())
        .context("open session store")
        .map_err(|err| mcp_error(err.to_string()))?;
    let sessions = store
        .list_active()
        .context("list sessions")
        .map_err(|err| mcp_error(err.to_string()))?;
    let filtered = if let Some(scope) = &globals.scope {
        sessions
            .into_iter()
            .filter(|session| session.scope.as_deref() == Some(scope.as_str()))
            .collect::<Vec<_>>()
    } else {
        sessions
    };
    Ok(json!(filtered))
}

fn json_response(response: &crate::repl::context::JsonResponse) -> Result<CallToolResult, McpError> {
    if response.ok {
        let value = response.result.clone().unwrap_or(Value::Null);
        text_result(&value)
    } else {
        Err(mcp_error(
            response
                .error
                .clone()
                .unwrap_or_else(|| "repl request failed".into()),
        ))
    }
}

fn text_result(value: &Value) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|err| mcp_error(format!("serialize result: {err}")))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn mcp_error(message: impl Into<String>) -> McpError {
    McpError::internal_error(message.into(), None)
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
struct EmptyParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StackTraceParams {
    #[schemars(description = "Thread id from debug_threads")]
    thread_id: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SetBreakpointsParams {
    #[schemars(description = "Source file path")]
    path: String,
    #[schemars(description = "Breakpoint line numbers")]
    lines: Vec<i64>,
    #[schemars(description = "Optional condition applied to every line")]
    condition: Option<String>,
    #[schemars(description = "Optional hit condition applied to every line")]
    hit_condition: Option<String>,
    #[schemars(description = "Optional log message applied to every line")]
    log_message: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BreakpointParams {
    #[schemars(description = "Source file path")]
    path: String,
    #[schemars(description = "Line number")]
    line: i64,
    #[schemars(description = "add, remove, or toggle (default: add)")]
    action: Option<String>,
    #[schemars(description = "Optional breakpoint condition")]
    condition: Option<String>,
    #[schemars(description = "Optional hit condition")]
    hit_condition: Option<String>,
    #[schemars(description = "Optional log message")]
    log_message: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ClearBreakpointsParams {
    #[schemars(description = "Source file path")]
    path: String,
    #[schemars(description = "Optional single line to clear; omit to clear all lines in the file")]
    line: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SkipParams {
    #[schemars(description = "list, add, or clear")]
    action: String,
    #[schemars(description = "Path pattern for add/clear")]
    pattern: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InstructionBreakpointParams {
    #[schemars(description = "Memory reference from disassembly")]
    memory_reference: String,
    #[schemars(description = "Instruction offset")]
    offset: Option<i64>,
    #[schemars(description = "Enable (true) or disable (false); default true")]
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BoolToggleParams {
    #[schemars(description = "true or false")]
    enabled: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ThreadParams {
    #[schemars(description = "Thread id")]
    thread_id: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FrameParams {
    #[schemars(description = "Stack frame id")]
    frame_id: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WaitForStopParams {
    #[schemars(description = "Maximum wait time in milliseconds (default 30000)")]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LaunchParams {
    #[schemars(description = "Program or binary to debug")]
    program: String,
    #[schemars(description = "Adapter id (e.g. fake, python, rust)")]
    adapter: Option<String>,
    #[schemars(description = "RSP target id from target.yaml")]
    target: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AttachParams {
    #[schemars(description = "Control attach port published by dap-proxy")]
    control_port: Option<u16>,
    #[schemars(description = "Instance id from debug_sessions")]
    instance_id: Option<String>,
    #[schemars(description = "Optional scope filter")]
    scope: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ShowParams {
    #[schemars(description = "Lines of context before/after the current line")]
    context_lines: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NavigateParams {
    #[schemars(description = "continue, step_over, step_in, step_out, pause, step_back, reverse_continue")]
    navigation_type: String,
    #[schemars(description = "Thread id to navigate")]
    thread_id: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EvaluateParams {
    #[schemars(description = "Expression to evaluate")]
    expression: String,
    #[schemars(description = "Optional stack frame id")]
    frame_id: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ScopesParams {
    #[schemars(description = "Stack frame id")]
    frame_id: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct VariablesParams {
    #[schemars(description = "Variables reference from scopes")]
    variables_reference: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SetVariableParams {
    #[schemars(description = "Variable name")]
    name: String,
    #[schemars(description = "New value")]
    value: String,
    #[schemars(description = "Optional scope reference")]
    variables_reference: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ExceptionBreakpointParams {
    #[schemars(description = "Exception filter id")]
    filter: Option<String>,
    #[schemars(description = "Optional condition expression")]
    condition: Option<String>,
    #[schemars(description = "add, remove, or clear (default: add)")]
    action: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReadMemoryParams {
    #[schemars(description = "Memory reference address")]
    memory_reference: String,
    #[schemars(description = "Bytes to read (default 256)")]
    count: Option<i64>,
    #[schemars(description = "Byte offset from memory reference")]
    offset: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WriteMemoryParams {
    #[schemars(description = "Memory reference address")]
    memory_reference: String,
    #[schemars(description = "Hex-encoded bytes to write")]
    data: String,
    #[schemars(description = "Byte offset from memory reference")]
    offset: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ThreadSnapshotParams {
    #[schemars(description = "Include stack traces (default true)")]
    include_stacks: Option<bool>,
    #[schemars(description = "Max stack frames per thread (default 10)")]
    stack_depth: Option<i64>,
    #[schemars(description = "Max threads to enumerate (default 50)")]
    max_threads: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DataWatchParams {
    #[schemars(description = "add, remove, or list")]
    action: String,
    #[schemars(description = "Watch expression for add/remove")]
    expression: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RestartFrameParams {
    #[schemars(description = "Stack frame to restart from")]
    frame_id: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FunctionBreakpointParams {
    #[schemars(description = "add, remove, or clear")]
    action: String,
    #[schemars(description = "Function name for add/remove")]
    name: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GotoParams {
    #[schemars(description = "Source file path")]
    path: String,
    #[schemars(description = "Target line number")]
    line: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WatchParams {
    #[schemars(description = "add, remove, or list")]
    action: String,
    #[schemars(description = "Watch expression for add/remove")]
    expression: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CompletionsParams {
    #[schemars(description = "Partial expression to complete")]
    expression: String,
    #[schemars(description = "Column offset within the expression")]
    column: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DisassembleParams {
    #[schemars(description = "Memory reference from the adapter")]
    memory_reference: String,
    #[schemars(description = "Instruction offset")]
    offset: Option<i64>,
    #[schemars(description = "Number of instructions to disassemble")]
    count: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DapRequestParams {
    #[schemars(description = "DAP command name")]
    command: String,
    #[schemars(description = "Optional JSON arguments object")]
    arguments: Option<Value>,
}
