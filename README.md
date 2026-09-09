# DAP Proxy

Debug Adapter Protocol gateway with pluggable backends, multi-client multiplexing, and reusable generic libraries.

## Layout

| Crate | Role |
|-------|------|
| [`instance-manager`](instance-manager/) | Generic instance registry (DAP, LSP, …) |
| [`protocol-gateway`](protocol-gateway/) | Plugin registry + routing + spawn specs |
| [`protocol-mux`](protocol-mux/) | N clients → 1 upstream, seq remapping |
| [`dap-protocol`](dap-protocol/) | DAP messages and Content-Length framing |
| [`dap-core`](dap-core/) | DAP sessions, router, engine |
| [`dap-plugin-api`](dap-plugin-api/) | Plugin manifest types |
| [`dap-proxy`](dap-proxy/) | Editor-facing binary (stdio) |
| [`dap-cli`](dap-cli/) | CLI control plane |
| [`dap-agent`](dap-agent/) | MCP server for AI agents (Go, NDJSON → `dap-cli`) |
| [`reference/dapper`](reference/dapper/) | Meta Dapper reference (MIT, not built) |

See [docs/architecture.md](docs/architecture.md) for data flow and phases.

## Build & test

```bash
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

## Status

**Phase 5** — navigation, evaluate/scopes/variables, rendered CLI output, `startDebugging` reverse requests.

```bash
dap-cli debug step-over 1
dap-cli debug evaluate "x" --frame-id 1
dap-cli debug scopes 1
```

### Rust debugging (`lldb-dap`)

Requires [`lldb-dap`](https://github.com/llvm/llvm-project/tree/main/lldb/tools/lldb-dap) on `PATH` (or set `LLDB_DAP` to its absolute path).

```bash
cargo build
# Auto-routes Cargo binaries under target/debug/ or target/release/
./target/debug/dap-proxy --program target/debug/myapp

# Or pick the rust plugin explicitly
./target/debug/dap-proxy --adapter rust --program target/debug/myapp

dap-cli plugin list
```

### Interactive debugger (gdb-style)

```bash
cargo build -p dap-cli -p fake-dap-adapter

# Local session with fake adapter (no lldb-dap needed for trying the REPL)
./target/debug/dap-cli debug repl --adapter fake main.py

# Rust/Cargo binary (requires lldb-dap on PATH)
cargo build -p dap-cli
./target/debug/dap-cli debug repl target/debug/dap-cli

# Attach to a running dap-proxy session instead (editor + REPL parallel)
./target/debug/dap-cli debug repl
```

See [Editor + REPL workflow](docs/editor-repl-workflow.md) for running `dap-proxy` in an editor and attaching the REPL from a terminal.

For a custom TUI or other DAP client over stdio, see [TUI client workflow](docs/tui-client.md).

Inside the REPL: `help`, `break main.rs:10`, `c`, `n`, `bt`, `locals`, `p expr`, `q` (attach mode: `q` detaches without ending the editor session).

**NDJSON mode** (`--ndjson` or global `--json`): one JSON object per line on stdin, one JSON response per line on stdout — suitable for scripts and tooling.

```bash
# Plain text (default)
dap-cli debug repl --adapter fake main.py

# NDJSON
printf '%s\n' '{"id":1,"op":"threads"}' '{"id":2,"op":"step_over"}' '{"id":3,"op":"quit"}' \
  | dap-cli debug repl --ndjson --adapter fake main.py
```

Example requests:

```json
{"id":1,"op":"breakpoint","path":"main.rs","line":10,"action":"toggle"}
{"id":2,"op":"thread","thread_id":1}
{"id":3,"op":"step_over"}
{"id":4,"op":"locals"}
{"id":5,"op":"evaluate","expression":"x + 1"}
{"id":6,"op":"quit"}
```

Example response:

```json
{"id":1,"ok":true,"result":{"path":"main.rs","line":10,"action":"toggle","enabled":true,"active_lines":[10],"breakpoints":{...}}}
```

For lldb-dap, `program` must be the **compiled binary** (not the `.rs` source). Headless init adds `sourceLanguages: ["rust"]` automatically.

```bash
cargo build -p dap-proxy -p dap-cli -p fake-dap-adapter
go build -o target/debug/dap-agent ./dap-agent

# MCP server for AI agents (stdio, talks to dap-cli over NDJSON repl)
./target/debug/dap-agent
# or: dap-cli debug mcp
```

**Phase 3** — CLI control plane + session discovery + gateway routing.

```bash
cargo build -p dap-proxy -p dap-cli -p fake-dap-adapter

# Proxy with auto-routing by file extension
./target/debug/dap-proxy --adapter-cmd ./target/debug/fake-dap-adapter --program main.py

# Discover sessions and send debug commands (no port needed when one session is active)
dap-cli session list
dap-cli debug sessions
dap-cli debug threads
dap-cli debug stack-trace 1
dap-cli debug set-breakpoints ./main.py -b 10
dap-cli debug dap threads --arguments '{}'
dap-cli plugin list
```

Global flags: `--json`, `--control-port`, `--scope` / `DAP_SCOPE_ID`. Sessions are stored under `$DAP_SESSIONS_DIR` or `$XDG_DATA_HOME/dap/sessions`.

**Phase 2** — multiplexed sessions: editor on stdio + control clients over TCP share one backend.

```bash
cargo build -p dap-proxy -p dap-cli -p fake-dap-adapter

# Start proxy (stderr prints {"controlPort":N})
./target/debug/dap-proxy --adapter-cmd ./target/debug/fake-dap-adapter 2>proxy.log &
# Editor speaks DAP on stdin/stdout; control clients attach via TCP:
./target/debug/dap-cli debug attach --port <N> --command threads
```

Flags: `--control-port 0` (ephemeral, default), `--no-control` (editor-only).

**Phase 1** — stdio proxy: `dap-proxy` forwards DAP to a spawned backend adapter (`fake`, `python` plugins).

```bash
# Dev: proxy → fake adapter
cargo build -p dap-proxy -p fake-dap-adapter
./target/debug/dap-proxy --adapter-cmd ./target/debug/fake-dap-adapter

# Or use builtin plugin id
./target/debug/dap-proxy --adapter fake
```

**Phase 0** — crate boundaries, generic mux/gateway libs, `fake-dap-adapter` fixture, CI.
