# dap-cli

Control plane CLI for the DAP proxy: session discovery, debug commands, gdb-style REPL, and built-in MCP server.

Depends on [`dap-core`](../dap-core/) (path dependencies during local development).

```bash
cargo build
cargo test

dap-cli session list
dap-cli plugin list
dap-cli debug start --program ./main.py
dap-cli debug repl --adapter fake main.py
```

## MCP (AI agents)

`dap-cli debug mcp` starts a built-in MCP server on stdio — no separate binary required.

Attach to a running session (from `dap-proxy`):

```json
{
  "mcpServers": {
    "dap": {
      "command": "/path/to/dap-cli",
      "args": ["debug", "mcp"]
    }
  }
}
```

Start a new debug session from the agent:

```json
{
  "mcpServers": {
    "dap": {
      "command": "/path/to/dap-cli",
      "args": ["debug", "mcp", "--adapter", "fake", "--program", "main.py"]
    }
  }
}
```

Use `--control-port` or `--scope` (global flags) to attach to a specific multiplexed session.

Key MCP tools for AI agents:

| Tool | Purpose |
|------|---------|
| `debug_attach` / `debug_launch` | Connect at runtime |
| `debug_inspect` | Status + stack + source + locals in one call |
| `debug_wait_for_stop` | Block until next stop after continue/step |
| `debug_breakpoint` | Single breakpoint with condition/log |
| `debug_terminate` | End session and kill debuggee |
