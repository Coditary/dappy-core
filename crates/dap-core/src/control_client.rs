use std::collections::VecDeque;

use anyhow::{Context, Result};
use dap_protocol::{Message, ReadChannel, WriteChannel};
use protocol_mux::BREAKPOINT_SNAPSHOT_COMMAND;
use serde_json::Value;
use tokio::time::Duration;

use crate::capabilities::AdapterCapabilities;
use crate::proxy_plugin::DAP_PROXY_PLUGIN_INFO_COMMAND;
use crate::{connect_control_client, roundtrip_request};

/// A source breakpoint sent to `setBreakpoints`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBreakpointSpec {
    pub line: i64,
    pub condition: Option<String>,
    pub hit_condition: Option<String>,
    pub log_message: Option<String>,
}

impl SourceBreakpointSpec {
    pub fn line(line: i64) -> Self {
        Self {
            line,
            condition: None,
            hit_condition: None,
            log_message: None,
        }
    }
}

/// An exception breakpoint filter sent to `setExceptionBreakpoints`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionBreakpointSpec {
    pub filter: String,
    pub condition: Option<String>,
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const LATE_JOIN_REPLAY_TIMEOUT: Duration = Duration::from_millis(200);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Control-plane client for a running multiplexed session.
pub struct ControlClient {
    read: ReadChannel,
    write: WriteChannel,
    seq: i64,
    pending: VecDeque<Message>,
    capabilities: AdapterCapabilities,
}

impl ControlClient {
    pub async fn connect(control_port: u16) -> Result<Self> {
        let duplex = tokio::time::timeout(CONNECT_TIMEOUT, connect_control_client(control_port))
            .await
            .with_context(|| format!("timeout connecting to control port {control_port}"))?
            .with_context(|| format!("connect control port {control_port}"))?;
        let (read, write) = duplex.into_channels();
        let mut client = Self {
            read,
            write,
            seq: 1,
            pending: VecDeque::new(),
            capabilities: AdapterCapabilities::unknown(),
        };
        client.absorb_late_join_replay().await?;
        Ok(client)
    }

    /// Connect directly to an adapter stdio duplex (bypasses the multiplex proxy).
    pub fn from_duplex(duplex: dap_protocol::DuplexChannel) -> Self {
        let (read, write) = duplex.into_channels();
        Self {
            read,
            write,
            seq: 1,
            pending: VecDeque::new(),
            capabilities: AdapterCapabilities::unknown(),
        }
    }

    pub fn capabilities(&self) -> &AdapterCapabilities {
        &self.capabilities
    }

    pub fn set_capabilities(&mut self, capabilities: AdapterCapabilities) {
        self.capabilities = capabilities;
    }

    /// Read the multiplexer's late-join replay (initialize response, events) and
    /// restore adapter capabilities for clients that attach after session init.
    async fn absorb_late_join_replay(&mut self) -> Result<()> {
        let messages = self.drain(LATE_JOIN_REPLAY_TIMEOUT).await?;
        for message in messages {
            if let Message::Response(response) = &message {
                if response.command.as_deref() == Some("initialize") {
                    if let Ok(caps) = AdapterCapabilities::from_initialize_message(&message) {
                        self.capabilities = caps;
                    }
                }
            }
            self.buffer_message(message);
        }
        Ok(())
    }

    async fn recv_next(&mut self) -> Result<Option<Message>> {
        if let Some(msg) = self.pending.pop_front() {
            return Ok(Some(msg));
        }
        self.recv_from_wire().await
    }

    /// Read the next message from the transport, ignoring `pending`.
    async fn recv_from_wire(&mut self) -> Result<Option<Message>> {
        self.read.recv().await.map_err(Into::into)
    }

    pub fn buffer_message(&mut self, message: Message) {
        self.pending.push_back(message);
    }

    /// Send a DAP request without waiting for its response.
    pub async fn send_dap_request(
        &mut self,
        command: &str,
        arguments: Option<Value>,
    ) -> Result<i64> {
        let seq = self.seq;
        self.seq += 1;
        let request = Message::Request(dap_protocol::Request {
            seq,
            command: command.to_string(),
            arguments,
        });
        self.write.send(&request).await?;
        Ok(seq)
    }

    /// Wait for a previously sent request's response, preserving other messages in `pending`.
    pub async fn send_dap_request_and_wait_for_event(
        &mut self,
        command: &str,
        arguments: Option<Value>,
        event_name: &str,
        timeout: Duration,
    ) -> Result<i64> {
        tokio::time::timeout(
            timeout,
            self.send_dap_request_and_wait_for_event_inner(command, arguments, event_name),
        )
        .await
        .with_context(|| format!("timeout waiting for event {event_name}"))?
    }

    async fn send_dap_request_and_wait_for_event_inner(
        &mut self,
        command: &str,
        arguments: Option<Value>,
        event_name: &str,
    ) -> Result<i64> {
        let seq = self.seq;
        self.seq += 1;
        let request = Message::Request(dap_protocol::Request {
            seq,
            command: command.to_string(),
            arguments,
        });
        self.write.send(&request).await?;
        while let Some(message) = self.recv_from_wire().await? {
            if let Message::Event(event) = &message {
                if event.event == event_name {
                    return Ok(seq);
                }
            }
            self.buffer_message(message);
        }
        anyhow::bail!("channel closed before event {event_name}")
    }

    pub async fn wait_for_response_seq(&mut self, seq: i64) -> Result<Message> {
        if let Some(index) = self
            .pending
            .iter()
            .position(|message| response_matches_seq(message, seq))
        {
            return Ok(self.pending.remove(index).expect("response index"));
        }

        while let Some(message) = self.recv_from_wire().await? {
            if response_matches_seq(&message, seq) {
                return Ok(message);
            }
            self.buffer_message(message);
        }
        anyhow::bail!("channel closed before response to seq {seq}")
    }

    pub async fn wait_for_response_named(&mut self, command: &str) -> Result<Message> {
        if let Some(index) = self
            .pending
            .iter()
            .position(|message| response_matches_command(message, command))
        {
            return Ok(self.pending.remove(index).expect("response index"));
        }

        while let Some(message) = self.recv_from_wire().await? {
            if response_matches_command(&message, command) {
                return Ok(message);
            }
            self.buffer_message(message);
        }
        anyhow::bail!("channel closed before response to {command}")
    }

    pub async fn dap_request(
        &mut self,
        command: &str,
        arguments: Option<Value>,
    ) -> Result<Message> {
        let seq = self.seq;
        self.seq += 1;
        tokio::time::timeout(
            REQUEST_TIMEOUT,
            roundtrip_request(&mut self.read, &mut self.write, seq, command, arguments),
        )
        .await
        .with_context(|| format!("timeout waiting for {command} response"))?
    }

    /// Like `dap_request`, but preserves interleaved events/responses in `pending`.
    pub async fn dap_request_preserve_events(
        &mut self,
        command: &str,
        arguments: Option<Value>,
    ) -> Result<Message> {
        tokio::time::timeout(
            REQUEST_TIMEOUT,
            self.dap_request_preserve_events_inner(command, arguments),
        )
        .await
        .with_context(|| format!("timeout waiting for {command} response"))?
    }

    async fn dap_request_preserve_events_inner(
        &mut self,
        command: &str,
        arguments: Option<Value>,
    ) -> Result<Message> {
        let seq = self.seq;
        self.seq += 1;
        let request = Message::Request(dap_protocol::Request {
            seq,
            command: command.to_string(),
            arguments,
        });
        self.write.send(&request).await?;
        while let Some(message) = self.recv_from_wire().await? {
            if let Message::Response(response) = &message {
                if response.request_seq == seq {
                    return Ok(message);
                }
            }
            self.pending.push_back(message);
        }
        anyhow::bail!("channel closed before response to {command}")
    }

    /// Read the next message, preferring any buffered events from prior requests.
    pub async fn read_message(&mut self) -> Result<Option<Message>> {
        self.recv_next().await
    }

    pub async fn threads(&mut self) -> Result<Value> {
        let message = self
            .dap_request("threads", Some(Value::Object(Default::default())))
            .await?;
        response_body(message)
    }

    pub async fn stack_trace(&mut self, thread_id: i64) -> Result<Value> {
        self.stack_trace_with_options(thread_id, None, None).await
    }

    pub async fn stack_trace_with_options(
        &mut self,
        thread_id: i64,
        start_frame: Option<i64>,
        levels: Option<i64>,
    ) -> Result<Value> {
        let mut args = serde_json::json!({ "threadId": thread_id });
        if let Some(start_frame) = start_frame {
            args["startFrame"] = serde_json::json!(start_frame);
        }
        if let Some(levels) = levels {
            args["levels"] = serde_json::json!(levels);
        }
        let message = self.dap_request("stackTrace", Some(args)).await?;
        response_body(message)
    }

    pub async fn read_memory(
        &mut self,
        memory_reference: &str,
        count: i64,
        offset: Option<i64>,
    ) -> Result<Value> {
        let mut args = serde_json::json!({
            "memoryReference": memory_reference,
            "count": count,
        });
        if let Some(offset) = offset {
            args["offset"] = serde_json::json!(offset);
        }
        let message = self.dap_request("readMemory", Some(args)).await?;
        response_body(message)
    }

    pub async fn write_memory(
        &mut self,
        memory_reference: &str,
        data_base64: &str,
        offset: Option<i64>,
        allow_partial: Option<bool>,
    ) -> Result<Value> {
        let mut args = serde_json::json!({
            "memoryReference": memory_reference,
            "data": data_base64,
        });
        if let Some(offset) = offset {
            args["offset"] = serde_json::json!(offset);
        }
        if let Some(allow_partial) = allow_partial {
            args["allowPartial"] = serde_json::json!(allow_partial);
        }
        let message = self.dap_request("writeMemory", Some(args)).await?;
        response_body(message)
    }

    pub async fn set_breakpoints(
        &mut self,
        source: &str,
        breakpoints: &[SourceBreakpointSpec],
    ) -> Result<Value> {
        let breakpoints = breakpoints
            .iter()
            .map(|bp| {
                let mut value = serde_json::json!({ "line": bp.line });
                if let Some(condition) = &bp.condition {
                    value["condition"] = serde_json::json!(condition);
                }
                if let Some(hit_condition) = &bp.hit_condition {
                    value["hitCondition"] = serde_json::json!(hit_condition);
                }
                if let Some(log_message) = &bp.log_message {
                    value["logMessage"] = serde_json::json!(log_message);
                }
                value
            })
            .collect::<Vec<_>>();
        let message = self
            .dap_request(
                "setBreakpoints",
                Some(serde_json::json!({
                    "source": { "path": source },
                    "breakpoints": breakpoints,
                })),
            )
            .await?;
        response_body(message)
    }

    pub async fn scopes(&mut self, frame_id: i64) -> Result<Value> {
        let message = self
            .dap_request("scopes", Some(serde_json::json!({ "frameId": frame_id })))
            .await?;
        response_body(message)
    }

    pub async fn variables(&mut self, variables_reference: i64) -> Result<Value> {
        let message = self
            .dap_request(
                "variables",
                Some(serde_json::json!({ "variablesReference": variables_reference })),
            )
            .await?;
        response_body(message)
    }

    pub async fn evaluate(&mut self, expression: &str, frame_id: Option<i64>) -> Result<Value> {
        let mut args = serde_json::json!({ "expression": expression });
        if let Some(frame_id) = frame_id {
            args["frameId"] = serde_json::json!(frame_id);
        }
        let message = self.dap_request("evaluate", Some(args)).await?;
        response_body(message)
    }

    pub async fn set_variable(
        &mut self,
        name: &str,
        value: &str,
        variables_reference: i64,
    ) -> Result<Value> {
        let message = self
            .dap_request(
                "setVariable",
                Some(serde_json::json!({
                    "variablesReference": variables_reference,
                    "name": name,
                    "value": value,
                })),
            )
            .await?;
        response_body(message)
    }

    pub async fn set_expression(
        &mut self,
        expression: &str,
        value: &str,
        frame_id: Option<i64>,
    ) -> Result<Value> {
        let mut args = serde_json::json!({
            "expression": expression,
            "value": value,
        });
        if let Some(frame_id) = frame_id {
            args["frameId"] = serde_json::json!(frame_id);
        }
        let message = self.dap_request("setExpression", Some(args)).await?;
        response_body(message)
    }

    pub async fn set_function_breakpoints(&mut self, names: &[String]) -> Result<Value> {
        let breakpoints = names
            .iter()
            .map(|name| serde_json::json!({ "name": name }))
            .collect::<Vec<_>>();
        let message = self
            .dap_request(
                "setFunctionBreakpoints",
                Some(serde_json::json!({ "breakpoints": breakpoints })),
            )
            .await?;
        response_body(message)
    }

    pub async fn breakpoint_locations(
        &mut self,
        source: &str,
        line: i64,
        end_line: Option<i64>,
    ) -> Result<Value> {
        let mut args = serde_json::json!({
            "source": { "path": source },
            "line": line,
        });
        if let Some(end_line) = end_line {
            args["endLine"] = serde_json::json!(end_line);
        }
        let message = self.dap_request("breakpointLocations", Some(args)).await?;
        response_body(message)
    }

    pub async fn goto_targets(&mut self, source: &str, line: i64) -> Result<Value> {
        let message = self
            .dap_request(
                "gotoTargets",
                Some(serde_json::json!({
                    "source": { "path": source },
                    "line": line,
                })),
            )
            .await?;
        response_body(message)
    }

    pub async fn goto(&mut self, thread_id: i64, target_id: i64) -> Result<Value> {
        let message = self
            .dap_request(
                "goto",
                Some(serde_json::json!({
                    "threadId": thread_id,
                    "targetId": target_id,
                })),
            )
            .await?;
        response_body(message)
    }

    pub async fn completions(
        &mut self,
        text: &str,
        column: i64,
        frame_id: Option<i64>,
    ) -> Result<Value> {
        let mut args = serde_json::json!({
            "text": text,
            "column": column,
        });
        if let Some(frame_id) = frame_id {
            args["frameId"] = serde_json::json!(frame_id);
        }
        let message = self.dap_request("completions", Some(args)).await?;
        response_body(message)
    }

    pub async fn restart_frame(&mut self, frame_id: i64) -> Result<Value> {
        let message = self
            .dap_request(
                "restartFrame",
                Some(serde_json::json!({ "frameId": frame_id })),
            )
            .await?;
        response_body(message)
    }

    pub async fn disassemble(
        &mut self,
        memory_reference: &str,
        instruction_offset: i64,
        instruction_count: i64,
    ) -> Result<Value> {
        let message = self
            .dap_request(
                "disassemble",
                Some(serde_json::json!({
                    "memoryReference": memory_reference,
                    "offset": instruction_offset,
                    "instructionCount": instruction_count,
                })),
            )
            .await?;
        response_body(message)
    }

    pub async fn set_instruction_breakpoints(
        &mut self,
        breakpoints: &[serde_json::Value],
    ) -> Result<Value> {
        let message = self
            .dap_request(
                "setInstructionBreakpoints",
                Some(serde_json::json!({ "breakpoints": breakpoints })),
            )
            .await?;
        response_body(message)
    }

    pub async fn set_data_breakpoints(
        &mut self,
        breakpoints: &[serde_json::Value],
    ) -> Result<Value> {
        let message = self
            .dap_request(
                "setDataBreakpoints",
                Some(serde_json::json!({ "breakpoints": breakpoints })),
            )
            .await?;
        response_body(message)
    }

    pub async fn set_exception_breakpoints(
        &mut self,
        breakpoints: &[ExceptionBreakpointSpec],
        supports_filter_options: bool,
    ) -> Result<Value> {
        let filters = breakpoints
            .iter()
            .map(|bp| bp.filter.clone())
            .collect::<Vec<_>>();
        let mut args = serde_json::json!({ "filters": filters });
        if supports_filter_options {
            let options = breakpoints
                .iter()
                .filter_map(|bp| {
                    bp.condition.as_ref().map(|condition| {
                        serde_json::json!({
                            "filterId": bp.filter,
                            "condition": condition,
                        })
                    })
                })
                .collect::<Vec<_>>();
            if !options.is_empty() {
                args["filterOptions"] = serde_json::json!(options);
            }
        }
        let message = self
            .dap_request("setExceptionBreakpoints", Some(args))
            .await?;
        response_body(message)
    }

    /// Fetch proxy plugin metadata (`dapProxyPluginInfo` extension).
    pub async fn proxy_plugin_info(&mut self) -> Result<Value> {
        let message = self
            .dap_request(DAP_PROXY_PLUGIN_INFO_COMMAND, None)
            .await?;
        response_body(message)
    }

    /// Fetch the multiplexer's tracked breakpoint state (editor + adapter).
    pub async fn session_breakpoint_snapshot(&mut self) -> Result<Value> {
        let message = self.dap_request(BREAKPOINT_SNAPSHOT_COMMAND, None).await?;
        response_body(message)
    }

    pub async fn navigate(
        &mut self,
        navigation_type: crate::navigation::NavigationType,
        thread_id: i64,
    ) -> Result<crate::navigation::NavigateResult> {
        use crate::execution_state::ExecutionStateTracker;
        use crate::navigation::NavigateResult;

        self.dap_request_preserve_events(
            navigation_type.dap_command(),
            Some(navigation_type.arguments(thread_id)),
        )
        .await?;

        let mut stop_reason = None;
        let mut stopped_thread = None;
        if navigation_type.waits_for_stop() {
            let stopped = self
                .wait_for_event("stopped", Duration::from_secs(30))
                .await?;
            if let Message::Event(event) = &stopped {
                stop_reason = event
                    .body
                    .as_ref()
                    .and_then(|b| b.get("reason"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                stopped_thread = event
                    .body
                    .as_ref()
                    .and_then(|b| b.get("threadId"))
                    .and_then(|v| v.as_i64());
            }
        } else {
            let mut tracker = ExecutionStateTracker::new();
            for message in self.drain(Duration::from_millis(200)).await? {
                tracker.apply_message(&message);
            }
            let summary = tracker.summary().state;
            stop_reason = summary.stop_reason;
            stopped_thread = summary.thread_id;
        }

        Ok(NavigateResult {
            navigation_type,
            success: true,
            stop_reason,
            thread_id: stopped_thread.or(Some(thread_id)),
        })
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        self.disconnect_with_options(false).await
    }

    /// Disconnect and optionally terminate the debuggee (`terminateDebuggee: true`).
    pub async fn terminate(&mut self) -> Result<()> {
        self.disconnect_with_options(true).await
    }

    async fn disconnect_with_options(&mut self, terminate_debuggee: bool) -> Result<()> {
        let arguments = serde_json::json!({ "terminateDebuggee": terminate_debuggee });
        let _ = tokio::time::timeout(
            DISCONNECT_TIMEOUT,
            self.dap_request("disconnect", Some(arguments)),
        )
        .await;
        Ok(())
    }

    /// Read the next inbound message, if any, within `timeout`.
    pub async fn recv_timeout(&mut self, timeout: Duration) -> Result<Option<Message>> {
        match tokio::time::timeout(timeout, self.recv_next()).await {
            Ok(result) => result,
            Err(_) => Ok(None),
        }
    }

    /// Drain inbound messages until `timeout` elapses, returning all received messages.
    ///
    /// Buffered messages (e.g. from `wait_for_event`) are returned immediately. When
    /// `timeout` is zero, no wire reads are performed after the buffer is empty.
    pub async fn drain(&mut self, timeout: Duration) -> Result<Vec<Message>> {
        let mut messages = Vec::new();
        while let Some(msg) = self.pending.pop_front() {
            messages.push(msg);
        }

        if timeout.is_zero() {
            return Ok(messages);
        }

        while let Some(msg) = self.recv_timeout(Duration::from_millis(50)).await? {
            messages.push(msg);
        }

        let late_wait = if messages.is_empty() {
            timeout
        } else {
            Duration::from_millis(10)
        };
        if let Some(msg) = self.recv_timeout(late_wait).await? {
            messages.push(msg);
            while let Some(msg) = self.recv_timeout(Duration::from_millis(10)).await? {
                messages.push(msg);
            }
        }
        Ok(messages)
    }

    /// Wait until an event with the given name arrives, preserving other messages.
    pub async fn wait_for_event(&mut self, name: &str, timeout: Duration) -> Result<Message> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(index) = self
                .pending
                .iter()
                .position(|message| event_named(message, name))
            {
                return Ok(self.pending.remove(index).expect("event index"));
            }

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }

            match tokio::time::timeout(remaining, self.recv_from_wire()).await {
                Ok(Ok(Some(msg))) => {
                    if event_named(&msg, name) {
                        return Ok(msg);
                    }
                    self.buffer_message(msg);
                }
                Ok(Ok(None)) => break,
                Ok(Err(err)) => return Err(err),
                Err(_) => break,
            }
        }
        anyhow::bail!("timeout waiting for event {name}")
    }

    /// Sync execution state by issuing `threads` when no events were observed.
    pub async fn probe_status(&mut self) -> Result<crate::execution_state::ExecutionStateTracker> {
        use crate::execution_state::ExecutionStateTracker;

        let mut tracker = ExecutionStateTracker::new();
        for message in self.drain(Duration::from_millis(200)).await? {
            tracker.apply_message(&message);
        }
        if tracker.summary().state.status == crate::execution_state::ExecutionStatus::Unknown {
            let _ = self.threads().await?;
            for message in self.drain(Duration::from_millis(100)).await? {
                tracker.apply_message(&message);
            }
        }
        Ok(tracker)
    }
}

fn response_body(message: Message) -> Result<Value> {
    match message {
        Message::Response(resp) => Ok(resp.body.unwrap_or(Value::Null)),
        other => anyhow::bail!("expected response, got {:?}", other),
    }
}

fn response_matches_seq(message: &Message, seq: i64) -> bool {
    matches!(
        message,
        Message::Response(response) if response.request_seq == seq
    )
}

fn response_matches_command(message: &Message, command: &str) -> bool {
    matches!(
        message,
        Message::Response(response) if response.command.as_deref() == Some(command)
    )
}

fn event_named(message: &Message, name: &str) -> bool {
    matches!(
        message,
        Message::Event(event) if event.event == name
    )
}

/// Resolve a unique active session control port from the session store.
pub fn resolve_control_port(
    store: &instance_manager::SessionStore,
    control_port: Option<u16>,
    scope: Option<&str>,
) -> Result<u16> {
    if let Some(port) = control_port {
        return Ok(port);
    }

    let active = store
        .list_active()
        .context("list active sessions")?
        .into_iter()
        .filter(|record| scope.is_none() || record.scope.as_deref() == scope)
        .collect::<Vec<_>>();

    match active.len() {
        0 => anyhow::bail!("no active debug session found"),
        1 => Ok(active[0].control_port),
        n => {
            let ids = active
                .iter()
                .map(|r| format!("{}:{}", r.instance_id, r.control_port))
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!("ambiguous: {n} active sessions ({ids}); pass --control-port or --scope")
        }
    }
}
