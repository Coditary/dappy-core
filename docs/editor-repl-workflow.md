# Editor + REPL parallel workflow

Run a debug session in your editor while controlling it from a terminal REPL — similar to [dap-mux](https://github.com/dap-mux/dap-mux).

## Overview

```
┌─────────────┐   DAP stdio    ┌────────────┐   DAP stdio   ┌─────────────┐
│   Editor    │◄──────────────►│  dap-proxy │◄─────────────►│   Adapter   │
│ (VS Code,   │                │  (mux)     │               │ (lldb-dap,  │
│  Helix, …)  │                └─────┬──────┘               │  debugpy)   │
└─────────────┘                      │ TCP control           └─────────────┘
                                     ▼
                              ┌─────────────┐
                              │ dap-cli     │
                              │ debug repl  │
                              └─────────────┘
```

- The **editor** owns the primary DAP session (initialize, launch, breakpoints UI).
- **dap-proxy** multiplexes one adapter to multiple clients.
- The **REPL** attaches via the control TCP port and sends DAP requests without disconnecting the editor.

## Quick start

### 1. Start the proxy (editor side)

Point your editor's debug adapter at `dap-proxy` instead of the raw adapter.

**VS Code** (`launch.json` fragment):

```json
{
  "type": "node",
  "request": "launch",
  "name": "via dap-proxy",
  "runtimeExecutable": "/path/to/dap-proxy",
  "runtimeArgs": ["--program", "${file}"],
  "console": "integratedTerminal"
}
```

For Python/Rust, use the matching `--adapter` / `--program` flags instead of `node`.

**gdb-remote / embedded** (spawn gdbserver from `target.yaml`, builtin adapter):

```json
{
  "runtimeExecutable": "/path/to/dap-proxy",
  "runtimeArgs": [
    "--stdio",
    "--target", "gdbserver",
    "--program", "${workspaceFolder}/build/app.elf"
  ]
}
```

`dap-proxy` starts the RSP backend (e.g. `gdbserver`), waits for the TCP port, and merges default `host`/`port`/`program` into editor `attach` requests. Use `rsp-attach` when gdbserver is already running (J-Link, SSH tunnel).

**Helix** (`languages.toml` / debug adapter config): set `command` to `dap-proxy` and pass `--program` to your binary.

On start, `dap-proxy` prints the control port on **stderr**:

```json
{"controlPort":45123}
```

Capture this port (or use `dap-cli debug sessions`).

### 2. Start debugging in the editor

Use the editor's normal **Start Debugging** flow. Wait until the session hits a breakpoint or entry stop.

### 3. Attach the REPL (terminal side)

```bash
# Auto-discover when exactly one session is active:
dap-cli debug repl

# Or pin the port from dap-proxy stderr:
dap-cli debug repl --control-port 45123

# NDJSON scripting mode:
dap-cli debug repl --ndjson --control-port 45123
```

The REPL loads adapter **capabilities** from the running session (late-join replay), so breakpoint emulation matches the real adapter.

### 4. Quit the REPL without killing the editor session

Press `q` or send `{"op":"quit"}` in NDJSON mode. The REPL **detaches** from the control port; it does **not** send DAP `disconnect`. The editor session keeps running.

To end the whole session, stop debugging from the editor or run:

```bash
dap-cli debug stop              # disconnect adapter, keep debuggee
dap-cli debug stop --terminate  # disconnect and kill debuggee
dap-cli session kill <id>       # same via instance id
```

## Session lifecycle

- `dap-proxy` writes a session file on start and removes it on **any** exit (normal, SIGINT/SIGTERM, panic via `Drop` guard).
- `dap-cli session list` prunes stale entries (dead PID, closed control port, corrupt JSON).
- `dap-cli session show <id>` reports reachability probes.
- Control-plane requests time out after 90s; connect attempts after 5s.
- When the editor (or other parent process) exits, `dap-proxy` detects parent death (`PR_SET_PDEATHSIG` + PPID polling) and shuts down the adapter.
- Closing the editor DAP connection (stdio EOF) detaches the editor client, sends `disconnect` to the adapter, and ends the proxy session.
- Optional `--client-idle-timeout-secs N` closes the editor client after N seconds without inbound DAP traffic (useful for TUIs; default `0` = disabled). See [TUI client workflow](tui-client.md).
- Starting `dap-proxy` prunes stale session files from the on-disk store.

## Owned vs attach mode

| Command | Mode | Use case |
|---------|------|----------|
| `dap-cli debug repl main.py` | **Owned** | Local headless session (no editor). Spawns adapter + proxy internally. |
| `dap-cli debug repl` | **Attach** | Join an existing `dap-proxy` session started by an editor. |

## Session discovery

Active sessions are stored under `$XDG_DATA_HOME/dap/sessions` (override with `DAP_SESSIONS_DIR`).

```bash
dap-cli debug sessions
# or
dap-cli session list
```

When multiple sessions run, pass `--control-port` or `--scope` / `DAP_SCOPE_ID`.

## Tips

- Use `sync` in the REPL after attaching to refresh stack, execution state, and breakpoints imported from the editor session.
- `capabilities` / `caps` shows adapter features (from the editor's initialize handshake).
- Conditional breakpoints, logpoints, and hit counts are emulated client-side when the adapter lacks native support.
