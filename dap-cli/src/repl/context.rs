use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use dap_core::{
    AdapterCapabilities, ControlClient, DataWatchEntry, ExceptionBreakpointSpec,
    ExecutionStateTracker, FrameLocation, NavigateResult, NavigationType, SourceBreakpointSpec,
    DEFAULT_MAX_VALUE_CHARS, DEFAULT_MAX_VARIABLES, build_sync_presentation,
    condition_result_is_true, data_watch_should_stop, ensure_navigation_supported,
    encode_write_payload, find_function_in_dirs, format_memory_read, frame_location,
    hex_string_to_bytes, hit_count_matches, instruction_breakpoint_key, path_matches_skip,
    resolve_control_port, resolve_function_line, resolve_instruction_location,
    smart_step_should_skip, source_paths_match, truncate_variables_response,
    uses_client_stop_policy, uses_emulated_data_breakpoints, uses_emulated_exception_condition,
    validate_read_count, validate_source_line, DEFAULT_READ_COUNT,
};
use dap_protocol::Message;
use instance_manager::SessionStore;
use serde::Serialize;
use serde_json::{Value, json};

use crate::commands::GlobalOpts;
use crate::repl::breakpoint::{BreakpointOptions, BreakpointRequest};
use crate::repl::cancel::SharedStartupKill;
use crate::repl::session::HeadlessSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointAction {
    Add,
    Remove,
    Toggle,
}

#[derive(Debug, Clone, Default)]
struct BreakpointEntry {
    condition: Option<String>,
    hit_condition: Option<String>,
    log_message: Option<String>,
    hit_count: u32,
    emulated_condition: bool,
    emulated_hit: bool,
    emulated_log: bool,
}

#[derive(Debug, Clone, Default)]
struct ExceptionBreakpointEntry {
    filter: String,
    condition: Option<String>,
    emulated_condition: bool,
}

#[derive(Debug, Clone)]
struct FunctionBreakpointEntry {
    name: String,
    resolved_path: Option<String>,
    resolved_line: Option<i64>,
    emulated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingGoto {
    path: String,
    line: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionBreakpointAction {
    Add,
    Remove,
    Clear,
}

#[derive(Debug, Clone, Default)]
struct StopResolution {
    auto_continues: u32,
    logs: Vec<String>,
    data_watch_hits: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExceptionBreakpointAction {
    Add,
    Remove,
    Clear,
}

pub struct ReplContext {
    backend: SessionBackend,
    current_thread: i64,
    current_frame: i64,
    breakpoints: HashMap<String, BTreeMap<i64, BreakpointEntry>>,
    editor_breakpoints: HashMap<String, BTreeMap<i64, BreakpointEntry>>,
    data_watches: Vec<DataWatchEntry>,
    exception_breakpoints: BTreeMap<String, ExceptionBreakpointEntry>,
    skip_paths: Vec<String>,
    watches: Vec<String>,
    smart_step: bool,
    suppress_entry_stop: bool,
    suppress_entry_done: bool,
    session_ready: bool,
    pending_breakpoints: Vec<BreakpointRequest>,
    function_breakpoints: BTreeMap<String, FunctionBreakpointEntry>,
    pending_goto: Option<PendingGoto>,
    emulated_instruction_breakpoints: HashMap<String, (String, i64)>,
    attach_snapshot: Option<Value>,
    execution: ExecutionStateTracker,
    capabilities: AdapterCapabilities,
    source_search_dirs: Vec<PathBuf>,
    last_location: Option<FrameLocation>,
    program: Option<String>,
    adapter_id: Option<String>,
}

const MAX_THREAD_SNAPSHOT_STACK_DEPTH: i64 = 512;
const MAX_THREAD_SNAPSHOT_THREADS: usize = 500;
const DEFAULT_THREAD_SNAPSHOT_STACK_DEPTH: i64 = 10;
const DEFAULT_THREAD_SNAPSHOT_MAX_THREADS: i64 = 50;

enum SessionBackend {
    Owned(HeadlessSession),
    Attached(ControlClient),
}

impl ReplContext {
    pub async fn connect(
        options: &crate::repl::ReplOptions,
        startup_kill: Option<Arc<SharedStartupKill>>,
    ) -> Result<Self> {
        let mut ctx = if let Some(program) = &options.program {
            let session =
                HeadlessSession::spawn(program, options.adapter.as_deref(), startup_kill).await?;
            Self::from_owned(session, program)
        } else {
            Self::from_attached(connect_client(&options.globals).await?)
        };
        ctx.session_ready = false;
        ctx.program = options.program.clone();
        ctx.adapter_id = options.adapter.clone();
        let skip_connect_stop_policies = match &ctx.backend {
            SessionBackend::Owned(session) => session.init.entry_location.is_some(),
            SessionBackend::Attached(_) => false,
        };
        match &ctx.backend {
            SessionBackend::Owned(_) => {
                ctx.seed_from_init().await?;
                if !skip_connect_stop_policies {
                    let _ = ctx.resolve_stop_policies().await?;
                }
            }
            SessionBackend::Attached(_) => {
                ctx.observe_pending(Duration::ZERO).await?;
                ctx.sync_context().await?;
            }
        };
        if ctx.is_attached() {
            ctx.import_session_breakpoints().await?;
            let _ = ctx.resolve_stop_policies().await?;
            ctx.refresh_attach_snapshot().await?;
        }
        ctx.flush_pending_breakpoints().await?;
        ctx.session_ready = true;
        Ok(ctx)
    }

    fn new(
        backend: SessionBackend,
        capabilities: AdapterCapabilities,
        source_search_dirs: Vec<PathBuf>,
    ) -> Self {
        Self {
            backend,
            current_thread: 1,
            current_frame: 1,
            breakpoints: HashMap::new(),
            editor_breakpoints: HashMap::new(),
            data_watches: Vec::new(),
            exception_breakpoints: BTreeMap::new(),
            skip_paths: default_skip_paths(),
            watches: Vec::new(),
            smart_step: true,
            suppress_entry_stop: true,
            suppress_entry_done: false,
            session_ready: true,
            pending_breakpoints: Vec::new(),
            function_breakpoints: BTreeMap::new(),
            pending_goto: None,
            emulated_instruction_breakpoints: HashMap::new(),
            attach_snapshot: None,
            execution: ExecutionStateTracker::new(),
            capabilities,
            source_search_dirs,
            last_location: None,
            program: None,
            adapter_id: None,
        }
    }

    pub fn repl_prompt(&self) -> String {
        if let Some(location) = &self.last_location {
            let file = std::path::Path::new(&location.path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| location.path.clone());
            format!("{}:{}› ", file, location.line)
        } else {
            "dap› ".to_string()
        }
    }

    fn remember_location_from_stack(&mut self, stack: &Value) {
        if let Some(frame) = stack
            .get("stackFrames")
            .and_then(Value::as_array)
            .and_then(|frames| frames.first())
        {
            if let Some(location) = frame_location(frame) {
                self.last_location = Some(location);
            }
        }
    }

    fn from_owned(session: HeadlessSession, program: &str) -> Self {
        let capabilities = session.client.capabilities().clone();
        let thread_id = session.init.thread_id;
        let stop_reason = session.init.stop_reason.clone();
        let mut ctx = Self::new(
            SessionBackend::Owned(session),
            capabilities,
            source_search_dirs_for_program(program),
        );
        if let Some(thread_id) = thread_id {
            ctx.current_thread = thread_id;
        }
        if stop_reason.is_some() {
            ctx.execution.apply_event(
                "stopped",
                Some(&json!({
                    "reason": stop_reason,
                    "threadId": ctx.current_thread,
                })),
            );
        }
        ctx
    }

    fn from_attached(client: ControlClient) -> Self {
        let capabilities = client.capabilities().clone();
        let mut ctx = Self::new(SessionBackend::Attached(client), capabilities, Vec::new());
        // Never auto-continue entry stops on attach; the editor owns session lifecycle.
        ctx.suppress_entry_stop = false;
        ctx
    }

    fn is_attached(&self) -> bool {
        matches!(self.backend, SessionBackend::Attached(_))
    }

    async fn import_session_breakpoints(&mut self) -> Result<()> {
        let snapshot = self.client_mut().session_breakpoint_snapshot().await?;
        self.apply_breakpoint_snapshot(&snapshot);
        Ok(())
    }

    fn apply_breakpoint_snapshot(&mut self, snapshot: &Value) {
        self.editor_breakpoints.clear();
        if let Some(files) = snapshot.get("breakpoints").and_then(Value::as_object) {
            for (path, entries) in files {
                if let Some(items) = entries.as_array() {
                    let map = self.editor_breakpoints.entry(path.clone()).or_default();
                    for entry in items {
                        let line = entry.get("line").and_then(Value::as_i64);
                        if line.is_none() {
                            continue;
                        }
                        let line = line.unwrap_or(0);
                        let policy = uses_client_stop_policy(
                            &self.capabilities,
                            entry.get("condition").and_then(Value::as_str),
                            entry.get("hit_condition").and_then(Value::as_str),
                            entry.get("log_message").and_then(Value::as_str),
                        );
                        map.insert(
                            line,
                            BreakpointEntry {
                                condition: entry
                                    .get("condition")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                                hit_condition: entry
                                    .get("hit_condition")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                                log_message: entry
                                    .get("log_message")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                                hit_count: 0,
                                emulated_condition: policy.condition,
                                emulated_hit: policy.hit,
                                emulated_log: policy.log,
                            },
                        );
                    }
                    if map.is_empty() {
                        self.editor_breakpoints.remove(path);
                    }
                }
            }
        }

        if let Some(entries) = snapshot
            .get("exception_breakpoints")
            .and_then(Value::as_array)
        {
            for entry in entries {
                let filter = entry
                    .get("filter")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if filter.is_empty() || self.exception_breakpoints.contains_key(filter) {
                    continue;
                }
                let condition = entry
                    .get("condition")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let emulated_condition = uses_emulated_exception_condition(
                    &self.capabilities,
                    condition.as_deref(),
                );
                self.exception_breakpoints.insert(
                    filter.to_string(),
                    ExceptionBreakpointEntry {
                        filter: filter.to_string(),
                        condition,
                        emulated_condition,
                    },
                );
            }
        }
    }

    async fn seed_from_init(&mut self) -> Result<()> {
        if let SessionBackend::Owned(session) = &self.backend {
            if let Some(entry) = &session.init.entry_location {
                self.last_location = Some(FrameLocation {
                    path: entry.path.clone(),
                    line: entry.line,
                    column: Some(1),
                    name: None,
                    frame_id: entry.frame_id,
                });
                if let Some(frame_id) = entry.frame_id {
                    self.current_frame = frame_id;
                }
                return Ok(());
            }
        }
        self.refresh_frame().await?;
        Ok(())
    }

    async fn sync_context(&mut self) -> Result<()> {
        let threads = self.client_mut().threads().await?;
        self.current_thread = threads["threads"]
            .as_array()
            .and_then(|items| items.first())
            .and_then(|thread| thread.get("id"))
            .and_then(Value::as_i64)
            .unwrap_or(1);
        self.refresh_frame().await?;
        Ok(())
    }

    pub fn ready_state(&self) -> Value {
        json!({
            "thread_id": self.current_thread,
            "frame_id": self.current_frame,
            "execution": self.execution.summary(),
            "breakpoints": self.breakpoints_snapshot(),
            "exception_breakpoints": self.exception_breakpoints_snapshot(),
            "skip_paths": self.skip_paths,
            "watches": self.watches,
            "smart_step": self.smart_step,
            "suppress_entry_stop": self.suppress_entry_stop,
            "function_breakpoints": self.function_breakpoints_snapshot(),
            "attach_snapshot": self.attach_snapshot.clone(),
            "capabilities": self.capabilities,
        })
    }

    pub fn capabilities(&self) -> &AdapterCapabilities {
        &self.capabilities
    }

    pub fn session_config(&self) -> Value {
        json!({
            "program": self.program,
            "adapter_id": self.adapter_id,
            "context": self.ready_state(),
        })
    }

    pub async fn read_memory(
        &mut self,
        memory_reference: &str,
        count: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Value> {
        if !self.capabilities.supports_read_memory_request {
            anyhow::bail!("adapter does not support readMemory");
        }
        let count = validate_read_count(count.unwrap_or(DEFAULT_READ_COUNT))?;
        let body = self
            .client_mut()
            .read_memory(memory_reference, count, offset)
            .await?;
        let formatted = format_memory_read(&body)?;
        Ok(json!({
            "body": body,
            "formatted": formatted,
        }))
    }

    pub async fn write_memory(
        &mut self,
        memory_reference: &str,
        data_hex: &str,
        offset: Option<i64>,
    ) -> Result<Value> {
        if !self.capabilities.supports_write_memory_request {
            anyhow::bail!("adapter does not support writeMemory");
        }
        let bytes = hex_string_to_bytes(data_hex)?;
        let body = self
            .client_mut()
            .write_memory(
                memory_reference,
                &encode_write_payload(&bytes),
                offset,
                None,
            )
            .await?;
        let written = body
            .get("bytesWritten")
            .and_then(Value::as_i64)
            .unwrap_or(bytes.len() as i64);
        Ok(json!({
            "body": body,
            "bytes_written": written,
            "memory_reference": memory_reference,
        }))
    }

    pub async fn thread_snapshot(
        &mut self,
        include_stacks: bool,
        stack_depth: Option<i64>,
        max_threads: Option<i64>,
    ) -> Result<Value> {
        let stack_depth = stack_depth
            .unwrap_or(DEFAULT_THREAD_SNAPSHOT_STACK_DEPTH)
            .clamp(1, MAX_THREAD_SNAPSHOT_STACK_DEPTH);
        let max_threads = (max_threads.unwrap_or(DEFAULT_THREAD_SNAPSHOT_MAX_THREADS).max(1) as usize)
            .min(MAX_THREAD_SNAPSHOT_THREADS);

        let threads_body = self.client_mut().threads().await?;
        let threads = threads_body
            .get("threads")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let total_thread_count = threads.len();
        let truncated = total_thread_count > max_threads;
        let visible_threads = threads.into_iter().take(max_threads).collect::<Vec<_>>();

        let mut entries = Vec::with_capacity(visible_threads.len());
        for thread in visible_threads {
            let thread_id = thread.get("id").and_then(Value::as_i64).unwrap_or(0);
            let mut entry = json!({
                "id": thread_id,
                "name": thread.get("name").cloned().unwrap_or(Value::Null),
            });
            if include_stacks {
                match self
                    .client_mut()
                    .stack_trace_with_options(thread_id, None, Some(stack_depth))
                    .await
                {
                    Ok(stack) => {
                        entry["stack_frames"] = stack
                            .get("stackFrames")
                            .cloned()
                            .unwrap_or(Value::Array(vec![]));
                    }
                    Err(err) => {
                        entry["stack_error"] = Value::String(err.to_string());
                    }
                }
            }
            entries.push(entry);
        }

        Ok(json!({
            "thread_count": entries.len(),
            "total_thread_count": total_thread_count,
            "truncated": truncated,
            "threads": entries,
        }))
    }

    pub async fn sync(&mut self) -> Result<Value> {
        self.observe_pending(Duration::from_millis(500)).await?;
        if self.is_attached() {
            self.import_session_breakpoints().await?;
        }
        self.sync_context().await?;
        let resolution = self.resolve_stop_policies().await?;
        let stack = self.stack().await?;
        let watches = self.refresh_watches().await?;
        let execution = self.execution.summary();
        let breakpoints = self.breakpoints_snapshot();
        let presentation = build_sync_presentation(
            &execution,
            &stack,
            &watches,
            &breakpoints,
            &self.data_watches_snapshot(),
        );
        Ok(json!({
            "execution": execution,
            "thread_id": self.current_thread,
            "frame_id": self.current_frame,
            "stack": stack,
            "breakpoints": breakpoints,
            "exception_breakpoints": self.exception_breakpoints_snapshot(),
            "skip_paths": self.skip_paths,
            "watches": watches,
            "capabilities": self.capabilities,
            "auto_continues": resolution.auto_continues,
            "logs": resolution.logs,
            "data_watch_hits": resolution.data_watch_hits,
            "presentation": presentation,
        }))
    }

    pub async fn navigate(&mut self, navigation_type: NavigationType) -> Result<Value> {
        ensure_navigation_supported(&self.capabilities, navigation_type)
            .map_err(anyhow::Error::msg)?;
        let thread_id = self.current_thread;
        let result = self.client_mut().navigate(navigation_type, thread_id).await?;
        // `navigate` already waited for `stopped`; only process buffered follow-up events.
        self.observe_pending(Duration::ZERO).await?;
        if let Some(thread_id) = result.thread_id {
            self.current_thread = thread_id;
        }
        let thread_id = self.current_thread;
        let stack = self.client_mut().stack_trace(thread_id).await?;
        if let Some(frame_id) = top_frame_id(&stack) {
            self.current_frame = frame_id;
        }
        self.remember_location_from_stack(&stack);
        let resolution = self.resolve_stop_policies_with_stack(&stack).await?;
        let watches = self.refresh_watches().await?;
        let execution = self.execution.summary();
        let presentation = build_sync_presentation(
            &execution,
            &stack,
            &watches,
            &self.breakpoints_snapshot(),
            &self.data_watches_snapshot(),
        );
        Ok(json!({
            "navigation": result,
            "stack": stack,
            "thread_id": self.current_thread,
            "frame_id": self.current_frame,
            "watches": watches,
            "execution": execution,
            "auto_continues": resolution.auto_continues,
            "logs": resolution.logs,
            "data_watch_hits": resolution.data_watch_hits,
            "presentation": presentation,
        }))
    }

    pub async fn threads(&mut self) -> Result<Value> {
        self.client_mut().threads().await
    }

    pub async fn stack(&mut self) -> Result<Value> {
        let thread_id = self.current_thread;
        let body = self.client_mut().stack_trace(thread_id).await?;
        if let Some(frame_id) = top_frame_id(&body) {
            self.current_frame = frame_id;
        }
        self.remember_location_from_stack(&body);
        Ok(body)
    }

    pub async fn show(&mut self, context_lines: Option<u32>, styled: bool) -> Result<Value> {
        use dap_core::{
            SourceShowOptions, TerminalStyle, format_source_show_styled, read_source_file_with_hints,
            select_stack_frame,
        };

        let thread_id = self.current_thread;
        let stack = self.client_mut().stack_trace(thread_id).await?;
        let frame = select_stack_frame(&stack, self.current_frame)
            .context("stack trace has no frames")?;
        let location = frame_location(&frame).context("current frame has no source location")?;
        if let Some(frame_id) = location.frame_id {
            self.current_frame = frame_id;
        }
        self.last_location = Some(location.clone());

        let source = read_source_file_with_hints(&location.path, &self.source_search_dirs);
        let options = SourceShowOptions {
            context_lines: context_lines
                .map(|value| value as usize)
                .unwrap_or(2),
        };
        let style = styled.then(TerminalStyle::detect);
        let display = format_source_show_styled(&location, source.as_deref(), options, style);

        Ok(json!({
            "path": location.path,
            "line": location.line,
            "column": location.column,
            "name": location.name,
            "frame_id": location.frame_id,
            "thread_id": thread_id,
            "source_available": source.is_some(),
            "display": display,
        }))
    }

    pub async fn status(&mut self) -> Result<Value> {
        self.observe_pending(Duration::from_millis(100)).await?;
        Ok(serde_json::to_value(self.execution.summary())?)
    }

    pub async fn breakpoint(&mut self, request: BreakpointRequest) -> Result<Value> {
        if !self.session_ready {
            self.pending_breakpoints.push(request);
            return Ok(json!({
                "queued": true,
                "path": self.pending_breakpoints.last().map(|bp| bp.path.clone()),
                "line": self.pending_breakpoints.last().map(|bp| bp.line),
                "pending_count": self.pending_breakpoints.len(),
            }));
        }
        self.apply_breakpoint(request).await
    }

    async fn flush_pending_breakpoints(&mut self) -> Result<()> {
        let pending = self.pending_breakpoints.drain(..).collect::<Vec<_>>();
        for request in pending {
            self.apply_breakpoint(request).await?;
        }
        Ok(())
    }

    async fn apply_breakpoint(&mut self, request: BreakpointRequest) -> Result<Value> {
        let warning = validate_source_line(&request.path, request.line);
        let policy = uses_client_stop_policy(
            &self.capabilities,
            request.options.condition.as_deref(),
            request.options.hit_condition.as_deref(),
            request.options.log_message.as_deref(),
        );

        let enabled = {
            let entries = self.breakpoints.entry(request.path.clone()).or_default();
            match request.action {
                BreakpointAction::Add => {
                    entries.insert(
                        request.line,
                        entry_from_options(&request.options, policy),
                    );
                    true
                }
                BreakpointAction::Remove => {
                    entries.remove(&request.line);
                    false
                }
                BreakpointAction::Toggle => {
                    if entries.remove(&request.line).is_some() {
                        false
                    } else {
                        entries.insert(
                            request.line,
                            entry_from_options(&request.options, policy),
                        );
                        true
                    }
                }
            }
        };

        if self.breakpoints.get(&request.path).map(|e| e.is_empty()).unwrap_or(false) {
            self.breakpoints.remove(&request.path);
        }

        let active = self.breakpoint_specs_for_path(&request.path);
        let body = self
            .client_mut()
            .set_breakpoints(&request.path, &active)
            .await?;

        Ok(json!({
            "path": request.path,
            "line": request.line,
            "action": breakpoint_action_name(request.action),
            "enabled": enabled,
            "condition": request.options.condition,
            "hit_condition": request.options.hit_condition,
            "log_message": request.options.log_message,
            "emulated": policy,
            "warning": warning,
            "active_breakpoints": self.breakpoints_snapshot_for_path(&request.path),
            "breakpoints": body,
        }))
    }

    pub async fn clear_breakpoints(&mut self, path: String, line: Option<i64>) -> Result<Value> {
        let removed = match line {
            Some(line) => {
                let removed = self
                    .breakpoints
                    .get_mut(&path)
                    .map(|entries| entries.remove(&line).is_some())
                    .unwrap_or(false);
                if self
                    .breakpoints
                    .get(&path)
                    .map(|entries| entries.is_empty())
                    .unwrap_or(false)
                {
                    self.breakpoints.remove(&path);
                }
                removed
            }
            None => self.breakpoints.remove(&path).is_some(),
        };
        let active = self.breakpoint_specs_for_path(&path);
        let body = self.client_mut().set_breakpoints(&path, &active).await?;
        Ok(json!({
            "path": path,
            "line": line,
            "removed": removed,
            "active_breakpoints": self.breakpoints_snapshot_for_path(&path),
            "breakpoints": body,
        }))
    }

    pub fn skip_list(&self) -> Value {
        json!({ "skip_paths": self.skip_paths })
    }

    pub fn skip_add(&mut self, pattern: String) -> Value {
        if !self.skip_paths.iter().any(|existing| existing == &pattern) {
            self.skip_paths.push(pattern.clone());
        }
        self.skip_list()
    }

    pub fn skip_clear(&mut self, pattern: Option<String>) -> Value {
        match pattern {
            Some(pattern) => {
                self.skip_paths.retain(|existing| existing != &pattern);
            }
            None => self.skip_paths.clear(),
        }
        self.skip_list()
    }

    pub async fn evaluate(&mut self, expression: String) -> Result<Value> {
        let frame_id = self.current_frame;
        self.client_mut()
            .evaluate(&expression, Some(frame_id))
            .await
    }

    pub async fn scopes(&mut self) -> Result<Value> {
        let frame_id = self.current_frame;
        self.client_mut().scopes(frame_id).await
    }

    pub async fn locals(&mut self) -> Result<Value> {
        let frame_id = self.current_frame;
        let scopes = self.client_mut().scopes(frame_id).await?;
        let variables_reference = locals_reference(&scopes)?;
        let variables = self
            .client_mut()
            .variables(variables_reference)
            .await?;
        let variables = truncate_variables_response(
            &variables,
            DEFAULT_MAX_VARIABLES,
            DEFAULT_MAX_VALUE_CHARS,
        );
        Ok(json!({
            "frame_id": self.current_frame,
            "scopes": scopes,
            "variables": variables,
        }))
    }

    pub async fn variables(&mut self, variables_reference: i64) -> Result<Value> {
        let body = self.client_mut().variables(variables_reference).await?;
        Ok(truncate_variables_response(
            &body,
            DEFAULT_MAX_VARIABLES,
            DEFAULT_MAX_VALUE_CHARS,
        ))
    }

    pub fn list_breakpoints(&self) -> Value {
        self.breakpoints_snapshot_for_map(&self.merged_breakpoint_map())
    }

    pub fn exception_filters(&self) -> Value {
        json!({
            "filters": self.capabilities.exception_breakpoint_filters.clone().unwrap_or_default(),
            "installed": self.exception_breakpoints_snapshot(),
        })
    }

    pub async fn set_variable(
        &mut self,
        name: String,
        value: String,
        variables_reference: Option<i64>,
    ) -> Result<Value> {
        let frame_id = self.current_frame;
        let variables_reference = match variables_reference {
            Some(reference) => reference,
            None => {
                let scopes = self.client_mut().scopes(frame_id).await?;
                locals_reference(&scopes)?
            }
        };

        if self.capabilities.supports_set_variable {
            let body = self
                .client_mut()
                .set_variable(&name, &value, variables_reference)
                .await?;
            return Ok(json!({
                "name": name,
                "value": value,
                "variables_reference": variables_reference,
                "method": "setVariable",
                "result": body,
            }));
        }

        if self.capabilities.supports_set_expression {
            let body = self
                .client_mut()
                .set_expression(&name, &value, Some(frame_id))
                .await?;
            return Ok(json!({
                "name": name,
                "value": value,
                "frame_id": frame_id,
                "method": "setExpression",
                "emulated": true,
                "result": body,
            }));
        }

        anyhow::bail!("adapter does not support setVariable or setExpression");
    }

    pub async fn completions(&mut self, text: String, column: i64) -> Result<Value> {
        if !self.capabilities.supports_completions_request {
            return Ok(json!({
                "supported": false,
                "targets": [],
            }));
        }
        let frame_id = self.current_frame;
        let body = self
            .client_mut()
            .completions(&text, column, Some(frame_id))
            .await?;
        Ok(json!({
            "supported": true,
            "body": body,
            "targets": body.get("targets").cloned().unwrap_or(Value::Array(vec![])),
        }))
    }

    pub async fn goto_targets(&mut self, path: String, line: i64) -> Result<Value> {
        if !self.capabilities.supports_goto_targets_request {
            return Ok(json!({
                "supported": false,
                "emulated": true,
                "targets": [{
                    "id": 1,
                    "label": format!("{path}:{line}"),
                    "line": line,
                    "endLine": line,
                }],
            }));
        }
        let body = self.client_mut().goto_targets(&path, line).await?;
        Ok(json!({
            "supported": true,
            "body": body,
            "targets": body.get("targets").cloned().unwrap_or(Value::Array(vec![])),
        }))
    }

    pub async fn goto_line(&mut self, path: String, line: i64) -> Result<Value> {
        if self.capabilities.supports_goto_request {
            let targets = self.client_mut().goto_targets(&path, line).await?;
            let target_id = targets
                .get("targets")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|target| target.get("id"))
                .and_then(Value::as_i64)
                .context("gotoTargets returned no targets")?;
            let thread_id = self.current_thread;
            let body = self.client_mut().goto(thread_id, target_id).await?;
            self.observe_pending(Duration::ZERO).await?;
            self.refresh_frame().await?;
            let stack = self.stack().await?;
            return Ok(json!({
                "method": "goto",
                "path": path,
                "line": line,
                "target_id": target_id,
                "body": body,
                "stack": stack,
                "execution": self.execution.summary(),
            }));
        }

        self.pending_goto = Some(PendingGoto {
            path: path.clone(),
            line,
        });
        self.apply_breakpoint(BreakpointRequest {
            path: path.clone(),
            line,
            action: BreakpointAction::Add,
            options: BreakpointOptions::default(),
        })
        .await?;
        let body = self.navigate(NavigationType::Continue).await?;
        Ok(json!({
            "method": "goto_emulated",
            "emulated": true,
            "path": path,
            "line": line,
            "navigation": body,
        }))
    }

    pub async fn function_breakpoint(
        &mut self,
        name: Option<String>,
        action: FunctionBreakpointAction,
    ) -> Result<Value> {
        let name_arg = name.clone();
        let removed = match action {
            FunctionBreakpointAction::Clear => {
                let names = self
                    .function_breakpoints
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>();
                let count = names.len();
                for function_name in names {
                    self.remove_function_breakpoint(&function_name).await?;
                }
                count > 0
            }
            FunctionBreakpointAction::Remove => {
                let function_name = name.context("remove requires function name")?;
                self.remove_function_breakpoint(&function_name).await?
            }
            FunctionBreakpointAction::Add => {
                let function_name = name.context("add requires function name")?;
                self.add_function_breakpoint(function_name).await?;
                true
            }
        };

        Ok(json!({
            "action": function_breakpoint_action_name(action),
            "name": name_arg,
            "removed": removed,
            "installed": self.function_breakpoints_snapshot(),
        }))
    }

    async fn add_function_breakpoint(&mut self, name: String) -> Result<()> {
        if self.capabilities.supports_function_breakpoints {
            self.client_mut()
                .set_function_breakpoints(&[name.clone()])
                .await?;
            self.function_breakpoints.insert(
                name.clone(),
                FunctionBreakpointEntry {
                    name,
                    resolved_path: None,
                    resolved_line: None,
                    emulated: false,
                },
            );
            return Ok(());
        }

        let (path, line) = self.resolve_function_location(&name)?;
        if self.capabilities.supports_breakpoint_locations_request {
            let _ = self
                .client_mut()
                .breakpoint_locations(&path, line, Some(line))
                .await?;
        }
        self.apply_breakpoint(BreakpointRequest {
            path: path.clone(),
            line,
            action: BreakpointAction::Add,
            options: BreakpointOptions::default(),
        })
        .await?;
        self.function_breakpoints.insert(
            name.clone(),
            FunctionBreakpointEntry {
                name,
                resolved_path: Some(path),
                resolved_line: Some(line),
                emulated: true,
            },
        );
        Ok(())
    }

    async fn remove_function_breakpoint(&mut self, name: &str) -> Result<bool> {
        let Some(entry) = self.function_breakpoints.remove(name) else {
            return Ok(false);
        };
        if entry.emulated {
            if let (Some(path), Some(line)) = (&entry.resolved_path, entry.resolved_line) {
                self.clear_breakpoints(path.clone(), Some(line)).await?;
            }
        } else {
            let remaining = self
                .function_breakpoints
                .values()
                .filter(|item| !item.emulated)
                .map(|item| item.name.clone())
                .collect::<Vec<_>>();
            self.client_mut()
                .set_function_breakpoints(&remaining)
                .await?;
        }
        Ok(true)
    }

    fn resolve_function_location(&self, name: &str) -> Result<(String, i64)> {
        if let Some(program) = &self.program {
            if let Ok(content) = std::fs::read_to_string(program) {
                if let Some(line) = resolve_function_line(&content, name) {
                    return Ok((program.clone(), line));
                }
            }
        }
        if let Some((path, line)) = find_function_in_dirs(name, &self.source_search_dirs) {
            return Ok((path.to_string_lossy().into_owned(), line));
        }
        anyhow::bail!("could not resolve function breakpoint for {name}");
    }

    pub async fn exception_breakpoint(
        &mut self,
        filter: Option<String>,
        condition: Option<String>,
        action: ExceptionBreakpointAction,
    ) -> Result<Value> {
        let filter_arg = filter.clone();
        let condition_arg = condition.clone();
        let removed = match action {
            ExceptionBreakpointAction::Clear => {
                let count = self.exception_breakpoints.len();
                self.exception_breakpoints.clear();
                count > 0
            }
            ExceptionBreakpointAction::Remove => {
                let filter = filter.context("remove requires filter")?;
                self.exception_breakpoints.remove(&filter).is_some()
            }
            ExceptionBreakpointAction::Add => {
                let filter = filter.context("catch requires filter")?;
                if self
                    .capabilities
                    .exception_breakpoint_filters
                    .as_ref()
                    .is_some_and(|filters| {
                        !filters.is_empty()
                            && !filters.iter().any(|entry| {
                                entry.get("filter").and_then(Value::as_str) == Some(&filter)
                            })
                    })
                {
                    anyhow::bail!("unknown exception filter: {filter}");
                }
                let emulated_condition = uses_emulated_exception_condition(
                    &self.capabilities,
                    condition.as_deref(),
                );
                self.exception_breakpoints.insert(
                    filter.clone(),
                    ExceptionBreakpointEntry {
                        filter,
                        condition,
                        emulated_condition,
                    },
                );
                true
            }
        };

        let body = self.sync_exception_breakpoints().await?;
        Ok(json!({
            "action": exception_breakpoint_action_name(action),
            "filter": filter_arg,
            "condition": condition_arg,
            "removed": removed,
            "installed": self.exception_breakpoints_snapshot(),
            "breakpoints": body,
        }))
    }

    pub async fn set_thread(&mut self, thread_id: i64) -> Result<Value> {
        self.current_thread = thread_id;
        self.refresh_frame().await?;
        Ok(json!({
            "thread_id": self.current_thread,
            "frame_id": self.current_frame,
        }))
    }

    pub async fn set_frame(&mut self, frame_id: i64) -> Result<Value> {
        self.current_frame = frame_id;
        let stack = self.stack().await?;
        Ok(json!({
            "frame_id": self.current_frame,
            "thread_id": self.current_thread,
            "stack": stack,
        }))
    }

    pub async fn dap_request(&mut self, command: &str, arguments: Option<Value>) -> Result<Value> {
        let message = self.client_mut().dap_request(command, arguments).await?;
        self.observe_message(&message);
        Ok(serde_json::to_value(message)?)
    }

    pub async fn shutdown(self) -> Result<()> {
        match self.backend {
            SessionBackend::Owned(session) => session.shutdown().await?,
            SessionBackend::Attached(_client) => {
                // Drop the TCP control connection without sending DAP `disconnect`,
                // so the editor session and debuggee keep running.
            }
        }
        Ok(())
    }

    fn client_mut(&mut self) -> &mut ControlClient {
        match &mut self.backend {
            SessionBackend::Owned(session) => &mut session.client,
            SessionBackend::Attached(client) => client,
        }
    }

    async fn refresh_frame(&mut self) -> Result<()> {
        let thread_id = self.current_thread;
        let body = self.client_mut().stack_trace(thread_id).await?;
        if let Some(frame_id) = top_frame_id(&body) {
            self.current_frame = frame_id;
        }
        self.remember_location_from_stack(&body);
        Ok(())
    }

    async fn observe_pending(&mut self, timeout: Duration) -> Result<()> {
        let messages = self.client_mut().drain(timeout).await?;
        for message in &messages {
            self.observe_message(message);
            if let Message::Response(response) = message {
                if response.command.as_deref() == Some("initialize") {
                    if let Ok(caps) = AdapterCapabilities::from_initialize_message(message) {
                        self.capabilities = caps;
                    }
                }
            }
        }
        Ok(())
    }

    fn observe_message(&mut self, message: &Message) {
        self.execution.apply_message(message);
    }

    fn apply_navigate_result(&mut self, result: &NavigateResult) {
        if let Some(thread_id) = result.thread_id {
            self.current_thread = thread_id;
        }
        if result.navigation_type.waits_for_stop() {
            self.execution.apply_event(
                "stopped",
                Some(&json!({
                    "reason": result.stop_reason,
                    "threadId": self.current_thread,
                })),
            );
        } else {
            self.execution.apply_event("continued", Some(&json!({ "threadId": self.current_thread })));
        }
    }

    fn breakpoint_specs_for_path(&self, path: &str) -> Vec<SourceBreakpointSpec> {
        let mut lines = BTreeMap::new();
        for map in [&self.breakpoints, &self.editor_breakpoints] {
            if let Some(entries) = map.get(path) {
                for (line, entry) in entries {
                    lines.insert(*line, entry);
                }
            }
        }
        for (stored_path, entries) in &self.breakpoints {
            if stored_path != path && source_paths_match(stored_path, path) {
                for (line, entry) in entries {
                    lines.insert(*line, entry);
                }
            }
        }
        for (stored_path, entries) in &self.editor_breakpoints {
            if stored_path != path && source_paths_match(stored_path, path) {
                for (line, entry) in entries {
                    lines.entry(*line).or_insert(entry);
                }
            }
        }
        lines
            .into_iter()
            .map(|(line, entry)| SourceBreakpointSpec {
                line,
                condition: if entry.emulated_condition {
                    None
                } else {
                    entry.condition.clone()
                },
                hit_condition: if entry.emulated_hit {
                    None
                } else {
                    entry.hit_condition.clone()
                },
                log_message: if entry.emulated_log {
                    None
                } else {
                    entry.log_message.clone()
                },
            })
            .collect()
    }

    async fn resolve_stop_policies(&mut self) -> Result<StopResolution> {
        let thread_id = self.current_thread;
        let stack = self.client_mut().stack_trace(thread_id).await?;
        self.resolve_stop_policies_with_stack(&stack).await
    }

    async fn resolve_stop_policies_with_stack(
        &mut self,
        stack: &Value,
    ) -> Result<StopResolution> {
        let mut resolution = StopResolution::default();
        let mut current_stack = stack.clone();
        for _ in 0..1000 {
            match self.evaluate_stop_action_from_stack(&current_stack).await? {
                StopAction::Stay => break,
                StopAction::Continue => {
                    let thread_id = self.current_thread;
                    let result = self
                        .client_mut()
                        .navigate(NavigationType::Continue, thread_id)
                        .await?;
                    self.apply_navigate_result(&result);
                    self.observe_pending(Duration::ZERO).await?;
                    self.refresh_frame().await?;
                    let thread_id = self.current_thread;
                    current_stack = self.client_mut().stack_trace(thread_id).await?;
                    resolution.auto_continues += 1;
                }
                StopAction::LogAndContinue { message } => {
                    resolution.logs.push(message);
                    let thread_id = self.current_thread;
                    let result = self
                        .client_mut()
                        .navigate(NavigationType::Continue, thread_id)
                        .await?;
                    self.apply_navigate_result(&result);
                    self.observe_pending(Duration::ZERO).await?;
                    self.refresh_frame().await?;
                    let thread_id = self.current_thread;
                    current_stack = self.client_mut().stack_trace(thread_id).await?;
                    resolution.auto_continues += 1;
                }
                StopAction::DataWatchHit { expression, value } => {
                    resolution.data_watch_hits.push(json!({
                        "expression": expression,
                        "value": value,
                    }));
                    break;
                }
            }
        }
        Ok(resolution)
    }

    async fn evaluate_stop_action(&mut self) -> Result<StopAction> {
        let thread_id = self.current_thread;
        let stack = self.client_mut().stack_trace(thread_id).await?;
        self.evaluate_stop_action_from_stack(&stack).await
    }

    async fn evaluate_stop_action_from_stack(&mut self, stack: &Value) -> Result<StopAction> {
        let frame_id = self.current_frame;
        let frame = stack["stackFrames"]
            .as_array()
            .and_then(|frames| frames.first())
            .context("stack trace missing frames")?;
        let path = frame
            .get("source")
            .and_then(|source| source.get("path"))
            .and_then(Value::as_str)
            .context("stack frame missing source path")?
            .to_string();
        let line = frame
            .get("line")
            .and_then(Value::as_i64)
            .context("stack frame missing line")?;

        let execution = self.execution.summary();
        let stop_reason = execution.state.stop_reason.as_deref();
        if self.suppress_entry_stop
            && !self.suppress_entry_done
            && stop_reason == Some("entry")
        {
            self.suppress_entry_done = true;
            return Ok(StopAction::Continue);
        }

        if stop_reason == Some("exception") {
            let emulated_conditions = self
                .exception_breakpoints
                .values()
                .filter(|entry| entry.emulated_condition)
                .filter_map(|entry| entry.condition.clone())
                .collect::<Vec<_>>();
            for expression in emulated_conditions {
                let body = self
                    .client_mut()
                    .evaluate(&expression, Some(frame_id))
                    .await?;
                if !condition_result_is_true(&body) {
                    return Ok(StopAction::Continue);
                }
            }
        }

        if let Some(target) = self.pending_goto.clone() {
            if source_paths_match(&target.path, &path) && target.line == line {
                self.pending_goto = None;
                let _ = self
                    .clear_breakpoints(target.path.clone(), Some(target.line))
                    .await;
                return Ok(StopAction::Stay);
            }
        }

        if let Some(hit) = self.evaluate_data_watches(frame_id).await? {
            return Ok(StopAction::DataWatchHit {
                expression: hit.0,
                value: hit.1,
            });
        }

        if smart_step_should_skip(&path, self.smart_step) {
            return Ok(StopAction::Continue);
        }

        if self
            .skip_paths
            .iter()
            .any(|pattern| path_matches_skip(&path, pattern))
        {
            return Ok(StopAction::Continue);
        }

        let Some((stored_path, line)) = self.stored_breakpoint_key(&path, line) else {
            return Ok(StopAction::Stay);
        };

        let snapshot = {
            let entry = self
                .breakpoint_entry_mut(&stored_path, line)
                .context("breakpoint entry disappeared")?;

            if entry.emulated_hit {
                entry.hit_count += 1;
                let hit_condition = entry.hit_condition.as_deref().unwrap_or("1");
                if !hit_count_matches(hit_condition, entry.hit_count) {
                    return Ok(StopAction::Continue);
                }
            }

            (
                entry.emulated_condition,
                entry.emulated_log,
                entry.condition.clone(),
                entry.log_message.clone(),
            )
        };

        let (emulated_condition, emulated_log, condition, log_message) = snapshot;

        if emulated_condition {
            let expression = condition.context("emulated condition missing expression")?;
            let body = self
                .client_mut()
                .evaluate(&expression, Some(frame_id))
                .await?;
            if !condition_result_is_true(&body) {
                return Ok(StopAction::Continue);
            }
        }

        if emulated_log {
            let template = log_message.context("emulated logpoint missing message")?;
            let message = self.format_log_message(&template).await?;
            return Ok(StopAction::LogAndContinue { message });
        }

        Ok(StopAction::Stay)
    }

    async fn format_log_message(&mut self, template: &str) -> Result<String> {
        let frame_id = self.current_frame;
        let mut out = String::new();
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            rest = &rest[start + 1..];
            match rest.find('}') {
                Some(end) => {
                    let expr = rest[..end].trim();
                    let body = self
                        .client_mut()
                        .evaluate(expr, Some(frame_id))
                        .await?;
                    let value = body
                        .get("result")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    out.push_str(value);
                    rest = &rest[end + 1..];
                }
                None => {
                    out.push('{');
                    break;
                }
            }
        }
        out.push_str(rest);
        Ok(out)
    }

    fn stored_breakpoint_key(&self, path: &str, line: i64) -> Option<(String, i64)> {
        for map in [&self.breakpoints, &self.editor_breakpoints] {
            for (stored_path, entries) in map {
                if source_paths_match(stored_path, path) && entries.contains_key(&line) {
                    return Some((stored_path.clone(), line));
                }
            }
        }
        None
    }

    fn breakpoint_entry_mut(
        &mut self,
        path: &str,
        line: i64,
    ) -> Option<&mut BreakpointEntry> {
        if let Some(entries) = self.breakpoints.get_mut(path) {
            if let Some(entry) = entries.get_mut(&line) {
                return Some(entry);
            }
        }
        self.editor_breakpoints
            .get_mut(path)
            .and_then(|entries| entries.get_mut(&line))
    }

    fn breakpoints_snapshot(&self) -> Value {
        json!({
            "repl": self.breakpoints_snapshot_for_map(&self.breakpoints),
            "editor": self.breakpoints_snapshot_for_map(&self.editor_breakpoints),
            "merged": self.breakpoints_snapshot_for_map(&self.merged_breakpoint_map()),
        })
    }

    fn merged_breakpoint_map(&self) -> HashMap<String, BTreeMap<i64, BreakpointEntry>> {
        let mut merged = self.editor_breakpoints.clone();
        for (path, entries) in &self.breakpoints {
            let map = merged.entry(path.clone()).or_default();
            for (line, entry) in entries {
                map.insert(*line, entry.clone());
            }
        }
        merged
    }

    fn breakpoints_snapshot_for_map(
        &self,
        map: &HashMap<String, BTreeMap<i64, BreakpointEntry>>,
    ) -> Value {
        let mut files = serde_json::Map::new();
        for path in map.keys() {
            files.insert(path.clone(), self.breakpoints_snapshot_for_path_in_map(map, path));
        }
        Value::Object(files)
    }

    fn breakpoints_snapshot_for_path(&self, path: &str) -> Value {
        self.breakpoints_snapshot_for_path_in_map(&self.breakpoints, path)
    }

    fn breakpoints_snapshot_for_path_in_map(
        &self,
        map: &HashMap<String, BTreeMap<i64, BreakpointEntry>>,
        path: &str,
    ) -> Value {
        match map.get(path) {
            Some(entries) => entries
                .iter()
                .map(|(line, entry)| {
                    json!({
                        "line": line,
                        "condition": entry.condition,
                        "hit_condition": entry.hit_condition,
                        "log_message": entry.log_message,
                        "hit_count": entry.hit_count,
                        "emulated": {
                            "condition": entry.emulated_condition,
                            "hit": entry.emulated_hit,
                            "log": entry.emulated_log,
                        },
                    })
                })
                .collect::<Vec<_>>()
                .into(),
            None => Value::Array(vec![]),
        }
    }

    async fn evaluate_data_watches(&mut self, frame_id: i64) -> Result<Option<(String, String)>> {
        if self.data_watches.is_empty()
            || !uses_emulated_data_breakpoints(self.capabilities.supports_data_breakpoints)
        {
            return Ok(None);
        }
        let expressions = self
            .data_watches
            .iter()
            .filter(|watch| watch.emulated)
            .map(|watch| watch.expression.clone())
            .collect::<Vec<_>>();
        for expression in expressions {
            let body = self
                .client_mut()
                .evaluate(&expression, Some(frame_id))
                .await?;
            let value = body
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let watch = self
                .data_watches
                .iter_mut()
                .find(|watch| watch.expression == expression)
                .expect("data watch entry");
            if data_watch_should_stop(watch, &value) {
                return Ok(Some((expression, value)));
            }
        }
        Ok(None)
    }

    pub async fn data_watch_add(&mut self, expression: String) -> Result<Value> {
        let emulated = uses_emulated_data_breakpoints(self.capabilities.supports_data_breakpoints);
        if !self.data_watches.iter().any(|watch| watch.expression == expression) {
            self.data_watches
                .push(DataWatchEntry::new(expression.clone(), emulated));
        }
        Ok(json!({
            "expressions": self.data_watches_snapshot(),
            "emulated": emulated,
        }))
    }

    pub async fn data_watch_remove(&mut self, expression: &str) -> Result<Value> {
        self.data_watches.retain(|watch| watch.expression != expression);
        Ok(json!({
            "removed": expression,
            "expressions": self.data_watches_snapshot(),
        }))
    }

    pub fn data_watch_list(&self) -> Value {
        json!({
            "watches": self.data_watches_snapshot(),
            "emulated": uses_emulated_data_breakpoints(self.capabilities.supports_data_breakpoints),
        })
    }

    fn data_watches_snapshot(&self) -> Value {
        self.data_watches
            .iter()
            .map(|watch| {
                json!({
                    "expression": watch.expression,
                    "baseline": watch.baseline,
                    "armed": watch.armed,
                    "emulated": watch.emulated,
                })
            })
            .collect::<Vec<_>>()
            .into()
    }

    pub async fn restart_frame(&mut self, frame_id: Option<i64>) -> Result<Value> {
        let frame_id = frame_id.unwrap_or(self.current_frame);
        if self.capabilities.supports_restart_frame {
            let body = self.client_mut().restart_frame(frame_id).await?;
            self.observe_pending(Duration::ZERO).await?;
            self.refresh_frame().await?;
            return Ok(json!({
                "method": "restartFrame",
                "frame_id": frame_id,
                "body": body,
                "execution": self.execution.summary(),
            }));
        }

        let thread_id = self.current_thread;
        let stack = self.client_mut().stack_trace(thread_id).await?;
        let frame = stack
            .get("stackFrames")
            .and_then(Value::as_array)
            .and_then(|frames| frames.iter().find(|f| f.get("id").and_then(Value::as_i64) == Some(frame_id)))
            .context("frame not found in stack trace")?;
        let path = frame
            .get("source")
            .and_then(|source| source.get("path"))
            .and_then(Value::as_str)
            .context("frame missing source path")?
            .to_string();
        let line = frame
            .get("line")
            .and_then(Value::as_i64)
            .context("frame missing line")?;
        let body = self.goto_line(path, line).await?;
        Ok(json!({
            "method": "restart_frame_emulated",
            "emulated": true,
            "frame_id": frame_id,
            "navigation": body,
        }))
    }

    pub async fn disassemble(
        &mut self,
        memory_reference: &str,
        offset: i64,
        count: i64,
    ) -> Result<Value> {
        if !self.capabilities.supports_disassemble_request {
            anyhow::bail!("adapter does not support disassemble");
        }
        let count = count.clamp(1, 256);
        let body = self
            .client_mut()
            .disassemble(memory_reference, offset, count)
            .await?;
        Ok(json!({
            "body": body,
            "memory_reference": memory_reference,
            "offset": offset,
            "count": count,
        }))
    }

    pub async fn instruction_breakpoint(
        &mut self,
        memory_reference: String,
        offset: i64,
        enabled: bool,
    ) -> Result<Value> {
        let key = instruction_breakpoint_key(&memory_reference, offset);
        if self.capabilities.supports_instruction_breakpoints {
            let breakpoints = if enabled {
                vec![json!({
                    "instructionReference": memory_reference,
                    "offset": offset,
                    "enabled": true,
                })]
            } else {
                vec![]
            };
            let body = self
                .client_mut()
                .set_instruction_breakpoints(&breakpoints)
                .await?;
            return Ok(json!({
                "method": "setInstructionBreakpoints",
                "enabled": enabled,
                "body": body,
            }));
        }
        if !self.capabilities.supports_disassemble_request {
            anyhow::bail!(
                "adapter does not support instruction breakpoints or disassemble"
            );
        }

        if !enabled {
            if let Some((path, line)) = self.emulated_instruction_breakpoints.remove(&key) {
                let body = self.clear_breakpoints(path, Some(line)).await?;
                return Ok(json!({
                    "method": "instruction_breakpoint_emulated",
                    "enabled": false,
                    "emulated": true,
                    "path": body["path"],
                    "line": line,
                }));
            }
            return Ok(json!({
                "method": "instruction_breakpoint_emulated",
                "enabled": false,
                "emulated": true,
                "removed": false,
            }));
        }

        let disasm = self
            .client_mut()
            .disassemble(&memory_reference, offset, 1)
            .await?;
        let (path, line) = resolve_instruction_location(&disasm)
            .context("disassemble response missing source location for emulated instruction breakpoint")?;
        let body = self
            .breakpoint(BreakpointRequest {
                path: path.clone(),
                line,
                action: BreakpointAction::Add,
                options: BreakpointOptions::default(),
            })
            .await?;
        let stored_path = body["path"]
            .as_str()
            .map(str::to_string)
            .unwrap_or(path);
        self.emulated_instruction_breakpoints
            .insert(key, (stored_path, line));
        Ok(json!({
            "method": "instruction_breakpoint_emulated",
            "enabled": true,
            "emulated": true,
            "path": body["path"],
            "line": line,
            "disassemble": disasm,
        }))
    }

    fn exception_breakpoints_snapshot(&self) -> Value {
        self.exception_breakpoints
            .values()
            .map(|entry| {
                json!({
                    "filter": entry.filter,
                    "condition": entry.condition,
                    "emulated": {
                        "condition": entry.emulated_condition,
                    },
                })
            })
            .collect::<Vec<_>>()
            .into()
    }

    async fn sync_exception_breakpoints(&mut self) -> Result<Value> {
        let specs = self
            .exception_breakpoints
            .values()
            .map(|entry| ExceptionBreakpointSpec {
                filter: entry.filter.clone(),
                condition: if entry.emulated_condition {
                    None
                } else {
                    entry.condition.clone()
                },
            })
            .collect::<Vec<_>>();
        let supports_filter_options = self.capabilities.supports_exception_filter_options;
        self.client_mut()
            .set_exception_breakpoints(&specs, supports_filter_options)
            .await
    }

    pub fn set_smart_step(&mut self, enabled: bool) -> Value {
        self.smart_step = enabled;
        json!({ "smart_step": self.smart_step })
    }

    pub fn set_suppress_entry_stop(&mut self, enabled: bool) -> Value {
        self.suppress_entry_stop = enabled;
        if !enabled {
            self.suppress_entry_done = false;
        }
        json!({ "suppress_entry_stop": self.suppress_entry_stop })
    }

    pub async fn watch_add(&mut self, expression: String) -> Result<Value> {
        if !self.watches.iter().any(|watch| watch == &expression) {
            self.watches.push(expression);
        }
        let watches = self.refresh_watches().await?;
        Ok(json!({
            "watches": watches,
            "expressions": self.watches,
        }))
    }

    pub async fn watch_remove(&mut self, expression: &str) -> Result<Value> {
        self.watches.retain(|watch| watch != expression);
        let watches = self.refresh_watches().await?;
        Ok(json!({
            "removed": expression,
            "watches": watches,
            "expressions": self.watches,
        }))
    }

    pub fn watch_list(&self) -> Value {
        json!({ "expressions": self.watches })
    }

    async fn refresh_attach_snapshot(&mut self) -> Result<()> {
        let stack = self.stack().await?;
        self.attach_snapshot = Some(json!({
            "thread_id": self.current_thread,
            "frame_id": self.current_frame,
            "execution": self.execution.summary(),
            "stack": stack,
            "breakpoints": self.breakpoints_snapshot(),
            "exception_breakpoints": self.exception_breakpoints_snapshot(),
        }));
        Ok(())
    }

    fn function_breakpoints_snapshot(&self) -> Value {
        self.function_breakpoints
            .values()
            .map(|entry| {
                json!({
                    "name": entry.name,
                    "resolved_path": entry.resolved_path,
                    "resolved_line": entry.resolved_line,
                    "emulated": entry.emulated,
                })
            })
            .collect::<Vec<_>>()
            .into()
    }

    async fn refresh_watches(&mut self) -> Result<Value> {
        let frame_id = self.current_frame;
        let expressions = self.watches.clone();
        let mut entries = Vec::new();
        for expression in expressions {
            let body = self
                .client_mut()
                .evaluate(&expression, Some(frame_id))
                .await?;
            entries.push(json!({
                "expression": expression,
                "result": body.get("result").cloned().unwrap_or(Value::Null),
                "type": body.get("type").cloned().unwrap_or(Value::Null),
            }));
        }
        Ok(Value::Array(entries))
    }
}

enum StopAction {
    Stay,
    Continue,
    LogAndContinue { message: String },
    DataWatchHit { expression: String, value: String },
}

fn entry_from_options(
    options: &BreakpointOptions,
    policy: dap_core::ClientStopPolicy,
) -> BreakpointEntry {
    BreakpointEntry {
        condition: options.condition.clone(),
        hit_condition: options.hit_condition.clone(),
        log_message: options.log_message.clone(),
        hit_count: 0,
        emulated_condition: policy.condition,
        emulated_hit: policy.hit,
        emulated_log: policy.log,
    }
}

fn default_skip_paths() -> Vec<String> {
    vec![
        "/rustc/".into(),
        "/target/deps/".into(),
        "site-packages".into(),
    ]
}

fn source_search_dirs_for_program(program: &str) -> Vec<PathBuf> {
    let path = std::path::Path::new(program);
    let mut dirs = Vec::new();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            dirs.push(parent.to_path_buf());
        }
    }
    if path.is_file() {
        if let Ok(abs) = path.canonicalize() {
            if let Some(parent) = abs.parent() {
                dirs.push(parent.to_path_buf());
            }
        }
    }
    dirs
}

fn breakpoint_action_name(action: BreakpointAction) -> &'static str {
    match action {
        BreakpointAction::Add => "add",
        BreakpointAction::Remove => "remove",
        BreakpointAction::Toggle => "toggle",
    }
}

fn function_breakpoint_action_name(action: FunctionBreakpointAction) -> &'static str {
    match action {
        FunctionBreakpointAction::Add => "add",
        FunctionBreakpointAction::Remove => "remove",
        FunctionBreakpointAction::Clear => "clear",
    }
}

fn exception_breakpoint_action_name(action: ExceptionBreakpointAction) -> &'static str {
    match action {
        ExceptionBreakpointAction::Add => "add",
        ExceptionBreakpointAction::Remove => "remove",
        ExceptionBreakpointAction::Clear => "clear",
    }
}

fn locals_reference(scopes: &Value) -> Result<i64> {
    scopes["scopes"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|scope| {
                scope
                    .get("name")
                    .and_then(Value::as_str)
                    .map(|name| name.eq_ignore_ascii_case("locals"))
                    .unwrap_or(false)
            })
        })
        .or_else(|| scopes["scopes"].as_array().and_then(|items| items.first()))
        .and_then(|scope| scope.get("variablesReference"))
        .and_then(Value::as_i64)
        .context("no scope with variables found")
}

fn top_frame_id(body: &Value) -> Option<i64> {
    body["stackFrames"]
        .as_array()
        .and_then(|frames| frames.first())
        .and_then(|frame| frame.get("id"))
        .and_then(Value::as_i64)
}

async fn connect_client(globals: &GlobalOpts) -> Result<ControlClient> {
    let store = SessionStore::open(SessionStore::default_dir())?;
    let port = resolve_control_port(&store, globals.control_port, globals.scope.as_deref())?;
    ControlClient::connect(port).await
}

#[derive(Serialize)]
pub struct JsonResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl JsonResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: Option<Value>, error: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(error.into()),
        }
    }

    pub fn to_line(&self) -> String {
        serde_json::to_string(self).expect("serialize json response")
    }
}

#[cfg(test)]
mod json_response_tests {
    use super::JsonResponse;
    use serde_json::json;

    #[test]
    fn serializes_success_and_failure() {
        let ok = JsonResponse::success(Some(json!(1)), json!({"ok": true}));
        let line = ok.to_line();
        assert!(line.contains("\"ok\":true"));
        assert!(line.contains("\"result\""));

        let err = JsonResponse::failure(Some(json!(2)), "boom");
        let line = err.to_line();
        assert!(line.contains("\"ok\":false"));
        assert!(line.contains("boom"));
    }
}
