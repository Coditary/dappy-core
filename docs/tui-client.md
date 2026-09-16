# TUI / custom DAP client workflow

Build a terminal UI (or any custom frontend) that speaks DAP over stdio to `dap-proxy`. The proxy routes to the correct adapter plugin, multiplexes optional control clients, and answers headless reverse requests (`runInTerminal`) on your behalf.

## Architecture

```
┌──────────────┐   DAP stdio    ┌────────────┐   DAP stdio   ┌─────────────┐
│  Your TUI    │◄──────────────►│  dap-proxy │◄─────────────►│   Adapter   │
│ (DAP client) │                │  (mux)     │               │ (debugpy,   │
└──────────────┘                └────────────┘               │  lldb-dap)  │
                                                              └─────────────┘
```

For a sole-client TUI, disable the control port so no second client can attach:

```bash
dap-proxy --stdio --no-control --program path/to/script.py
```

## Rust helpers (`dap-core`)

The [`DapClient`](https://github.com/coditary/dap) type (alias for `ControlClient`) is a buffered DAP client over any duplex transport.

### Spawn proxy and run session init

```rust
use dap_core::{
    ProxyStdioOptions, SessionInitConfig, spawn_proxy_stdio, run_session_init,
};

let mut opts = ProxyStdioOptions::program("main.py");
opts.proxy_bin = Some("/path/to/dap-proxy".into()); // or rely on PATH

let mut session = spawn_proxy_stdio(opts).await?;
let config = SessionInitConfig::new("main.py").with_adapter_id("python");

let ready = run_session_init(&mut session.client, &config).await?;
// ready.stop_reason, ready.thread_id, ready.capabilities
```

### Manual DAP sequence

If you prefer full control, use `DapClient` directly after `spawn_proxy_stdio`:

1. `initialize` — set `supportsRunInTerminalRequest: true` (proxy handles it headlessly)
2. `launch` / `attach`
3. wait for `initialized` event
4. `configurationDone`
5. wait for `stopped` event
6. `threads`, `stackTrace`, `scopes`, `variables`, `continue`, …

Use `build_initialize_arguments` / `build_launch_arguments` from `dap-core` for adapter-aware payloads.

### Connect to control port (optional)

When the control listener is enabled, a second client can attach in parallel (REPL, agent):

```rust
use dap_core::{ControlClient, run_session_init, SessionInitConfig};

let client = ControlClient::connect(control_port).await?;
```

## CLI flags for TUIs

| Flag | Purpose |
|------|---------|
| `--stdio` | Required: DAP over stdin/stdout |
| `--no-control` | Sole-client mode (no TCP attach) |
| `--program` | Adapter routing (`.py` → python, Cargo binary → rust) |
| `--client-idle-timeout-secs N` | Close session after N seconds without inbound DAP messages (`0` = disabled). Recommended `30` for TUIs that may crash without closing stdio. |
| `--adapter` / `--adapter-cmd` | Override plugin routing |

## Reverse requests

| Request | Proxy behavior |
|---------|----------------|
| `runInTerminal` | Spawns the process; stdout/stderr go to `$TMPDIR/dap-run-in-terminal/<seq>.log`. Response includes `logPath`. |
| `startDebugging` | Declined unless `--child-sessions` is set |

Declare `supportsRunInTerminalRequest: true` in your `initialize` request so adapters use the proxy path instead of blocking.

## Session discovery

When a control port is enabled, `dap-proxy` prints on stderr:

```json
{"controlPort":45123,"instanceId":"..."}
```

Sessions are registered under `$XDG_DATA_HOME/dap/sessions`. Stale files are pruned when a new proxy starts or when `dap-cli session list` runs.

## Lifecycle / cleanup

- Parent process exit → `dap-proxy` shuts down the adapter (`PR_SET_PDEATHSIG` + PPID watch).
- DAP stdio EOF or client idle timeout → editor disconnect → adapter `disconnect`.
- Proxy exit → session file removed via `SessionCleanupGuard`.

## Emulation vs native adapter

Breakpoint conditions, logpoints, smart step, and client-side data watches are implemented in `dap-cli` REPL / NDJSON control plane, not in the stdio mux path. A pure DAP TUI gets native adapter capabilities only. Attach the control port + REPL or NDJSON API when you need emulation.

## See also

- [Editor + REPL workflow](editor-repl-workflow.md) — multiplexed editor + terminal control
- [Architecture](architecture.md) — gateway and plugin model
