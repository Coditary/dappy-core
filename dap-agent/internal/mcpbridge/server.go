package mcpbridge

import (
	"context"
	"encoding/json"
	"fmt"
	"sync"

	"github.com/coditary/dap/dap-agent/internal/config"
	"github.com/coditary/dap/dap-agent/internal/repl"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// Server exposes DAP debug tools via MCP, backed by dap-cli NDJSON repl.
type Server struct {
	cfg    config.Config
	repl   *repl.Client
	replMu sync.Mutex
}

func New(cfg config.Config) *Server {
	return &Server{cfg: cfg}
}

func (s *Server) Register(mcpServer *mcp.Server) {
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_sessions",
		Description: "List active debug sessions from the session store.",
	}, s.debugSessions)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_status",
		Description: "Get current execution status from the debug session.",
	}, s.debugStatus)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_threads",
		Description: "List threads in the active debug session.",
	}, s.debugThreads)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_stack_trace",
		Description: "Fetch stack trace for a thread.",
	}, s.debugStackTrace)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_set_breakpoints",
		Description: "Set breakpoints in a source file (line numbers).",
	}, s.debugSetBreakpoints)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_breakpoints",
		Description: "List breakpoints tracked by the repl (includes editor breakpoints after attach/sync).",
	}, s.debugBreakpoints)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_sync",
		Description: "Refresh stack, execution state, and imported breakpoints from the session.",
	}, s.debugSync)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_show",
		Description: "Show source code around the current stack frame.",
	}, s.debugShow)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_navigate",
		Description: "Navigate execution (continue, step_over, step_in, step_out, pause, step_back, reverse_continue).",
	}, s.debugNavigate)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_evaluate",
		Description: "Evaluate an expression in the debugger.",
	}, s.debugEvaluate)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_scopes",
		Description: "List variable scopes for a stack frame.",
	}, s.debugScopes)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_variables",
		Description: "List variables for a variables reference.",
	}, s.debugVariables)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_set_variable",
		Description: "Set a variable in the current frame.",
	}, s.debugSetVariable)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_exception_breakpoint",
		Description: "Manage exception breakpoints (action: add, remove, clear).",
	}, s.debugExceptionBreakpoint)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_capabilities",
		Description: "Query adapter capabilities (supportsStepBack, readMemory, etc.).",
	}, s.debugCapabilities)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_config",
		Description: "Get session metadata (program, adapter, current context).",
	}, s.debugConfig)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_read_memory",
		Description: "Read memory from the debugged process at a memory reference.",
	}, s.debugReadMemory)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_write_memory",
		Description: "Write hex-encoded bytes to process memory at a memory reference.",
	}, s.debugWriteMemory)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_thread_snapshot",
		Description: "Snapshot all threads (optionally with stack traces) in one call.",
	}, s.debugThreadSnapshot)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_data_watch",
		Description: "Add, remove, or list emulated data watches (action: add, remove, list).",
	}, s.debugDataWatch)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_restart_frame",
		Description: "Restart execution from a stack frame (native or emulated via goto).",
	}, s.debugRestartFrame)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_function_breakpoint",
		Description: "Add, remove, or clear function breakpoints (action: add, remove, clear).",
	}, s.debugFunctionBreakpoint)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_goto",
		Description: "Jump execution to a source line (native or emulated).",
	}, s.debugGoto)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_watch",
		Description: "Add, remove, or list client-side watches (action: add, remove, list).",
	}, s.debugWatch)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_completions",
		Description: "Request completions for an expression at a column offset.",
	}, s.debugCompletions)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_disassemble",
		Description: "Disassemble instructions at a memory reference.",
	}, s.debugDisassemble)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_dap_request",
		Description: "Send a raw DAP request through the repl (op: dap).",
	}, s.debugDapRequest)
	mcp.AddTool(mcpServer, &mcp.Tool{
		Name:        "debug_stop",
		Description: "Disconnect from the debug session (repl quit).",
	}, s.debugStop)
}

func (s *Server) Close() {
	s.replMu.Lock()
	defer s.replMu.Unlock()
	if s.repl != nil {
		_ = s.repl.Close()
		s.repl = nil
	}
}

func (s *Server) withRepl(ctx context.Context) (*repl.Client, error) {
	s.replMu.Lock()
	defer s.replMu.Unlock()
	if s.repl != nil {
		return s.repl, nil
	}
	client, err := repl.Start(ctx, repl.Options{
		CLIPath:     s.cfg.CLIPath,
		ControlPort: s.cfg.ControlPort,
		Scope:       s.cfg.Scope,
		Program:     s.cfg.Program,
		Adapter:     s.cfg.Adapter,
	})
	if err != nil {
		return nil, err
	}
	s.repl = client
	return client, nil
}

func (s *Server) debugSessions(ctx context.Context, _ *mcp.CallToolRequest, _ struct{}) (*mcp.CallToolResult, string, error) {
	out, err := repl.RunCLI(ctx, s.cfg.CLIPath, []string{"--json", "debug", "sessions"}, scopeEnv(s.cfg.Scope))
	if err != nil {
		return nil, "", err
	}
	return nil, string(out), nil
}

func (s *Server) debugStatus(ctx context.Context, _ *mcp.CallToolRequest, _ struct{}) (*mcp.CallToolResult, string, error) {
	return s.opJSON(ctx, "status", nil)
}

func (s *Server) debugThreads(ctx context.Context, _ *mcp.CallToolRequest, _ struct{}) (*mcp.CallToolResult, string, error) {
	return s.opJSON(ctx, "threads", nil)
}

type stackTraceParams struct {
	ThreadID int64 `json:"thread_id" jsonschema:"Thread id from debug_threads"`
}

func (s *Server) debugStackTrace(ctx context.Context, _ *mcp.CallToolRequest, in stackTraceParams) (*mcp.CallToolResult, string, error) {
	client, err := s.withRepl(ctx)
	if err != nil {
		return nil, "", err
	}
	if in.ThreadID != 0 {
		if _, err := client.Op(ctx, "thread", map[string]any{"thread_id": in.ThreadID}); err != nil {
			return nil, "", err
		}
	}
	result, err := client.Op(ctx, "stack", nil)
	if err != nil {
		return nil, "", err
	}
	return nil, string(result), nil
}

type setBreakpointsParams struct {
	Path  string  `json:"path" jsonschema:"Source file path"`
	Lines []int64 `json:"lines" jsonschema:"Breakpoint line numbers"`
}

func (s *Server) debugSetBreakpoints(ctx context.Context, _ *mcp.CallToolRequest, in setBreakpointsParams) (*mcp.CallToolResult, string, error) {
	client, err := s.withRepl(ctx)
	if err != nil {
		return nil, "", err
	}
	breakpoints := make([]map[string]any, 0, len(in.Lines))
	for _, line := range in.Lines {
		breakpoints = append(breakpoints, map[string]any{"line": line})
	}
	result, err := client.Op(ctx, "dap", map[string]any{
		"command": "setBreakpoints",
		"arguments": map[string]any{
			"source":      map[string]any{"path": in.Path},
			"breakpoints": breakpoints,
		},
	})
	if err != nil {
		return nil, "", err
	}
	return nil, string(result), nil
}

func (s *Server) debugBreakpoints(ctx context.Context, _ *mcp.CallToolRequest, _ struct{}) (*mcp.CallToolResult, string, error) {
	return s.opJSON(ctx, "breakpoints", nil)
}

func (s *Server) debugSync(ctx context.Context, _ *mcp.CallToolRequest, _ struct{}) (*mcp.CallToolResult, string, error) {
	return s.opJSON(ctx, "sync", nil)
}

type showParams struct {
	ContextLines *uint32 `json:"context_lines,omitempty" jsonschema:"Lines of context before/after the current line"`
}

func (s *Server) debugShow(ctx context.Context, _ *mcp.CallToolRequest, in showParams) (*mcp.CallToolResult, string, error) {
	fields := map[string]any{}
	if in.ContextLines != nil {
		fields["context_lines"] = *in.ContextLines
	}
	return s.opJSON(ctx, "show", fields)
}

type navigateParams struct {
	NavigationType string `json:"navigation_type" jsonschema:"continue, step_over, step_in, step_out, pause, step_back, reverse_continue"`
	ThreadID       int64  `json:"thread_id" jsonschema:"Thread id to navigate"`
}

func (s *Server) debugNavigate(ctx context.Context, _ *mcp.CallToolRequest, in navigateParams) (*mcp.CallToolResult, string, error) {
	client, err := s.withRepl(ctx)
	if err != nil {
		return nil, "", err
	}
	if in.ThreadID != 0 {
		if _, err := client.Op(ctx, "thread", map[string]any{"thread_id": in.ThreadID}); err != nil {
			return nil, "", err
		}
	}
	result, err := client.Op(ctx, in.NavigationType, nil)
	if err != nil {
		return nil, "", err
	}
	return nil, string(result), nil
}

type evaluateParams struct {
	Expression string `json:"expression" jsonschema:"Expression to evaluate"`
	FrameID    *int64 `json:"frame_id,omitempty" jsonschema:"Optional stack frame id"`
}

func (s *Server) debugEvaluate(ctx context.Context, _ *mcp.CallToolRequest, in evaluateParams) (*mcp.CallToolResult, string, error) {
	client, err := s.withRepl(ctx)
	if err != nil {
		return nil, "", err
	}
	if in.FrameID != nil {
		if _, err := client.Op(ctx, "frame", map[string]any{"frame_id": *in.FrameID}); err != nil {
			return nil, "", err
		}
	}
	result, err := client.Op(ctx, "evaluate", map[string]any{"expression": in.Expression})
	if err != nil {
		return nil, "", err
	}
	return nil, string(result), nil
}

type scopesParams struct {
	FrameID int64 `json:"frame_id" jsonschema:"Stack frame id"`
}

func (s *Server) debugScopes(ctx context.Context, _ *mcp.CallToolRequest, in scopesParams) (*mcp.CallToolResult, string, error) {
	client, err := s.withRepl(ctx)
	if err != nil {
		return nil, "", err
	}
	if in.FrameID != 0 {
		if _, err := client.Op(ctx, "frame", map[string]any{"frame_id": in.FrameID}); err != nil {
			return nil, "", err
		}
	}
	result, err := client.Op(ctx, "scopes", nil)
	if err != nil {
		return nil, "", err
	}
	return nil, string(result), nil
}

type variablesParams struct {
	VariablesReference int64 `json:"variables_reference" jsonschema:"Variables reference from scopes"`
}

func (s *Server) debugVariables(ctx context.Context, _ *mcp.CallToolRequest, in variablesParams) (*mcp.CallToolResult, string, error) {
	return s.opJSON(ctx, "variables", map[string]any{"variables_reference": in.VariablesReference})
}

type setVariableParams struct {
	Name                string `json:"name" jsonschema:"Variable name"`
	Value               string `json:"value" jsonschema:"New value"`
	VariablesReference  *int64 `json:"variables_reference,omitempty" jsonschema:"Optional scope reference"`
}

func (s *Server) debugSetVariable(ctx context.Context, _ *mcp.CallToolRequest, in setVariableParams) (*mcp.CallToolResult, string, error) {
	fields := map[string]any{"name": in.Name, "value": in.Value}
	if in.VariablesReference != nil {
		fields["variables_reference"] = *in.VariablesReference
	}
	return s.opJSON(ctx, "set_variable", fields)
}

type exceptionBreakpointParams struct {
	Filter    *string `json:"filter,omitempty" jsonschema:"Exception filter id"`
	Condition *string `json:"condition,omitempty" jsonschema:"Optional condition expression"`
	Action    *string `json:"action,omitempty" jsonschema:"add, remove, or clear (default: add)"`
}

func (s *Server) debugExceptionBreakpoint(ctx context.Context, _ *mcp.CallToolRequest, in exceptionBreakpointParams) (*mcp.CallToolResult, string, error) {
	fields := map[string]any{}
	if in.Filter != nil {
		fields["filter"] = *in.Filter
	}
	if in.Condition != nil {
		fields["condition"] = *in.Condition
	}
	if in.Action != nil {
		fields["action"] = *in.Action
	}
	return s.opJSON(ctx, "exception_breakpoint", fields)
}

type dapRequestParams struct {
	Command   string          `json:"command" jsonschema:"DAP command name"`
	Arguments json.RawMessage `json:"arguments,omitempty" jsonschema:"Optional JSON arguments object"`
}

func (s *Server) debugCapabilities(ctx context.Context, _ *mcp.CallToolRequest, _ struct{}) (*mcp.CallToolResult, string, error) {
	return s.opJSON(ctx, "capabilities", nil)
}

func (s *Server) debugConfig(ctx context.Context, _ *mcp.CallToolRequest, _ struct{}) (*mcp.CallToolResult, string, error) {
	return s.opJSON(ctx, "config", nil)
}

type readMemoryParams struct {
	MemoryReference string `json:"memory_reference" jsonschema:"Memory reference address"`
	Count           *int64 `json:"count,omitempty" jsonschema:"Bytes to read (default 256)"`
	Offset          *int64 `json:"offset,omitempty" jsonschema:"Byte offset from memory reference"`
}

func (s *Server) debugReadMemory(ctx context.Context, _ *mcp.CallToolRequest, in readMemoryParams) (*mcp.CallToolResult, string, error) {
	fields := map[string]any{"memory_reference": in.MemoryReference}
	if in.Count != nil {
		fields["count"] = *in.Count
	}
	if in.Offset != nil {
		fields["offset"] = *in.Offset
	}
	return s.opJSON(ctx, "read_memory", fields)
}

type writeMemoryParams struct {
	MemoryReference string `json:"memory_reference" jsonschema:"Memory reference address"`
	Data            string `json:"data" jsonschema:"Hex-encoded bytes to write"`
	Offset          *int64 `json:"offset,omitempty" jsonschema:"Byte offset from memory reference"`
}

func (s *Server) debugWriteMemory(ctx context.Context, _ *mcp.CallToolRequest, in writeMemoryParams) (*mcp.CallToolResult, string, error) {
	fields := map[string]any{
		"memory_reference": in.MemoryReference,
		"data":             in.Data,
	}
	if in.Offset != nil {
		fields["offset"] = *in.Offset
	}
	return s.opJSON(ctx, "write_memory", fields)
}

type threadSnapshotParams struct {
	IncludeStacks *bool  `json:"include_stacks,omitempty" jsonschema:"Include stack traces (default true)"`
	StackDepth      *int64 `json:"stack_depth,omitempty" jsonschema:"Max stack frames per thread (default 10)"`
	MaxThreads      *int64 `json:"max_threads,omitempty" jsonschema:"Max threads to enumerate (default 50)"`
}

func (s *Server) debugThreadSnapshot(ctx context.Context, _ *mcp.CallToolRequest, in threadSnapshotParams) (*mcp.CallToolResult, string, error) {
	fields := map[string]any{}
	if in.IncludeStacks != nil {
		fields["include_stacks"] = *in.IncludeStacks
	}
	if in.StackDepth != nil {
		fields["stack_depth"] = *in.StackDepth
	}
	if in.MaxThreads != nil {
		fields["max_threads"] = *in.MaxThreads
	}
	return s.opJSON(ctx, "thread_snapshot", fields)
}

func (s *Server) debugDapRequest(ctx context.Context, _ *mcp.CallToolRequest, in dapRequestParams) (*mcp.CallToolResult, string, error) {
	fields := map[string]any{"command": in.Command}
	if len(in.Arguments) > 0 && string(in.Arguments) != "null" {
		var args any
		if err := json.Unmarshal(in.Arguments, &args); err != nil {
			return nil, "", fmt.Errorf("parse arguments: %w", err)
		}
		fields["arguments"] = args
	}
	return s.opJSON(ctx, "dap", fields)
}

type dataWatchParams struct {
	Action     string `json:"action" jsonschema:"add, remove, or list"`
	Expression string `json:"expression,omitempty" jsonschema:"Watch expression for add/remove"`
}

func (s *Server) debugDataWatch(ctx context.Context, _ *mcp.CallToolRequest, in dataWatchParams) (*mcp.CallToolResult, string, error) {
	switch in.Action {
	case "add":
		return s.opJSON(ctx, "data_watch_add", map[string]any{"expression": in.Expression})
	case "remove":
		return s.opJSON(ctx, "data_watch_remove", map[string]any{"expression": in.Expression})
	case "list", "":
		return s.opJSON(ctx, "data_watch_list", nil)
	default:
		return nil, "", fmt.Errorf("unknown data watch action: %s", in.Action)
	}
}

type restartFrameParams struct {
	FrameID *int64 `json:"frame_id,omitempty" jsonschema:"Stack frame to restart from"`
}

func (s *Server) debugRestartFrame(ctx context.Context, _ *mcp.CallToolRequest, in restartFrameParams) (*mcp.CallToolResult, string, error) {
	fields := map[string]any{}
	if in.FrameID != nil {
		fields["frame_id"] = *in.FrameID
	}
	return s.opJSON(ctx, "restart_frame", fields)
}

type functionBreakpointParams struct {
	Action string  `json:"action" jsonschema:"add, remove, or clear"`
	Name   *string `json:"name,omitempty" jsonschema:"Function name for add/remove"`
}

func (s *Server) debugFunctionBreakpoint(ctx context.Context, _ *mcp.CallToolRequest, in functionBreakpointParams) (*mcp.CallToolResult, string, error) {
	fields := map[string]any{"action": in.Action}
	if in.Name != nil {
		fields["name"] = *in.Name
	}
	return s.opJSON(ctx, "function_breakpoint", fields)
}

type gotoParams struct {
	Path string `json:"path" jsonschema:"Source file path"`
	Line int64  `json:"line" jsonschema:"Target line number"`
}

func (s *Server) debugGoto(ctx context.Context, _ *mcp.CallToolRequest, in gotoParams) (*mcp.CallToolResult, string, error) {
	return s.opJSON(ctx, "goto", map[string]any{
		"path": in.Path,
		"line": in.Line,
	})
}

type watchParams struct {
	Action     string `json:"action" jsonschema:"add, remove, or list"`
	Expression string `json:"expression,omitempty" jsonschema:"Watch expression for add/remove"`
}

func (s *Server) debugWatch(ctx context.Context, _ *mcp.CallToolRequest, in watchParams) (*mcp.CallToolResult, string, error) {
	switch in.Action {
	case "add":
		return s.opJSON(ctx, "watch_add", map[string]any{"expression": in.Expression})
	case "remove":
		return s.opJSON(ctx, "watch_remove", map[string]any{"expression": in.Expression})
	case "list", "":
		return s.opJSON(ctx, "watch_list", nil)
	default:
		return nil, "", fmt.Errorf("unknown watch action: %s", in.Action)
	}
}

type completionsParams struct {
	Expression string `json:"expression" jsonschema:"Partial expression to complete"`
	Column     *int64 `json:"column,omitempty" jsonschema:"Column offset within the expression"`
}

func (s *Server) debugCompletions(ctx context.Context, _ *mcp.CallToolRequest, in completionsParams) (*mcp.CallToolResult, string, error) {
	fields := map[string]any{"expression": in.Expression}
	if in.Column != nil {
		fields["column"] = *in.Column
	}
	return s.opJSON(ctx, "completions", fields)
}

type disassembleParams struct {
	MemoryReference string `json:"memory_reference" jsonschema:"Memory reference from the adapter"`
	Offset          *int64 `json:"offset,omitempty" jsonschema:"Instruction offset"`
	Count           *int64 `json:"count,omitempty" jsonschema:"Number of instructions to disassemble"`
}

func (s *Server) debugDisassemble(ctx context.Context, _ *mcp.CallToolRequest, in disassembleParams) (*mcp.CallToolResult, string, error) {
	fields := map[string]any{"memory_reference": in.MemoryReference}
	if in.Offset != nil {
		fields["offset"] = *in.Offset
	}
	if in.Count != nil {
		fields["count"] = *in.Count
	}
	return s.opJSON(ctx, "disassemble", fields)
}

func (s *Server) debugStop(ctx context.Context, _ *mcp.CallToolRequest, _ struct{}) (*mcp.CallToolResult, string, error) {
	s.replMu.Lock()
	client := s.repl
	s.repl = nil
	s.replMu.Unlock()

	if client == nil {
		return nil, `{"disconnected":true}`, nil
	}
	result, err := client.Op(ctx, "quit", nil)
	_ = client.Close()
	if err != nil {
		return nil, "", err
	}
	return nil, string(result), nil
}

func (s *Server) opJSON(ctx context.Context, op string, fields map[string]any) (*mcp.CallToolResult, string, error) {
	client, err := s.withRepl(ctx)
	if err != nil {
		return nil, "", err
	}
	result, err := client.Op(ctx, op, fields)
	if err != nil {
		return nil, "", err
	}
	return nil, string(result), nil
}

func scopeEnv(scope string) []string {
	if scope == "" {
		return nil
	}
	return []string{"DAP_SCOPE_ID=" + scope}
}
