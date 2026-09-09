# Architecture

This document describes the crate boundaries and data flow for the DAP proxy workspace.

## Goals

- **Gateway**: one entry point routes to many backend adapters (DAP debugpy, lldb-dap, …).
- **Multiplex**: many clients (editor, CLI, agent) share one live debug session.
- **Reuse**: generic crates work for LSP and other protocols later.

## Crate map

```
┌─────────────────────────────────────────────────────────────────┐
│ Binaries                                                         │
│  dap-proxy (editor stdio)    dap-cli (control)    dap-agent (*)  │
└────────────┬──────────────────────────┬──────────────────────────┘
             │                          │
┌────────────▼──────────────────────────▼──────────────────────────┐
│ dap-core — DAP sessions, init sequence, proxy orchestration       │
└────────────┬───────────────────────────────┬──────────────────────┘
             │                               │
┌────────────▼────────────┐    ┌─────────────▼─────────────┐
│ dap-protocol            │    │ dap-plugin-api            │
│ DAP JSON + framing      │    │ manifests, spawn metadata │
└─────────────────────────┘    └───────────────────────────┘
             │
┌────────────▼────────────────────────────────────────────────────┐
│ Generic (protocol-agnostic)                                      │
│  protocol-mux      N clients → 1 upstream, seq remap, broadcast  │
│  protocol-gateway  plugin registry + routing + SpawnSpec           │
│  instance-manager  instance registry, state, scope, parent/child   │
└──────────────────────────────────────────────────────────────────┘

(*) dap-agent — Go MCP server; stdio MCP ↔ `dap-cli debug repl --ndjson`

reference/dapper — vendored Meta Dapper (MIT), not built as part of workspace
```

## Session data flow (Phase 3)

```
dap-proxy (start)
    └── SessionStore.save(instance_id, pid, control_port, …)
            └── $XDG_DATA_HOME/dap/sessions/*.json

dap-cli debug threads
    └── SessionStore.list_active() → resolve_control_port()
            └── ControlClient → TCP control attach → multiplexed session
```

`dap-core::resolve_route` delegates to `protocol-gateway` using plugin `fileExtensions` and `launchTypes`.

## Session data flow (Phase 2)

```
Editor (stdio) ──┐
                 ├──► dap-core::mux_bridge ──► protocol-mux ──► Backend adapter
Control (TCP)  ──┘         ▲                        │
                           │                        └── broadcast events, per-client responses
                    controlPort on stderr
```

`dap-core::start_multiplexed_proxy` bridges `dap-protocol::Message` ↔ JSON for `protocol-mux`, runs the editor on stdio, and accepts control clients on `127.0.0.1:{control_port}`.

## Session data flow (full target)

```
Editor/CLI/Agent
       │  DAP requests
       ▼
┌──────────────┐
│ protocol-mux │  attach client, remap seq, broadcast events
└──────┬───────┘
       │
┌──────▼───────┐
│  dap-core    │  initialize / launch / configurationDone
└──────┬───────┘
       │ resolve plugin
┌──────▼───────────┐
│ protocol-gateway │
└──────┬───────────┘
       │ spawn stdio/tcp
┌──────▼───────────┐
│ Backend adapter  │  debugpy, lldb-dap, …
└──────────────────┘
```

## Instance model

`instance-manager` tracks every running hub object:

| Field | Purpose |
|-------|---------|
| `id` | Unique instance id |
| `kind` | `dap-session`, `lsp-server`, … |
| `scope` | Group sessions (workspace, agent id) |
| `control_port` | CLI/agent attach port |
| `parent_id` | Child session (DAP `startDebugging`) |
| `state` | `pending` → `starting` → `running` → … |

## Testing strategy

| Layer | Location |
|-------|----------|
| Unit | `*/tests/*.rs`, `#[cfg(test)]` in crates |
| Protocol | `dap-protocol/tests/` |
| Gateway/Mux | `protocol-gateway/tests/`, `protocol-mux/tests/` |
| Integration | `fake-dap-adapter/tests/initialize_e2e.rs` |
| E2E (later) | real debugpy / lldb, `#[ignore]` in CI |

## Phases

See project README.

| Phase | Focus |
|-------|--------|
| 0 | Crates, docs, CI, test fixtures |
| 1 | End-to-end DAP: `dap-proxy` stdio → spawned adapter (`dap-core::Backend`, transparent forward) |
| 2 | `protocol-mux` integrated: editor stdio + TCP control attach (`dap-core::mux_bridge`, `dap-cli debug attach`) |
| 3 | CLI control plane (`SessionStore`, `ControlClient`), gateway routing via `protocol-gateway`, `fileExtensions` in manifests |
| 4 | `session_init`, `ExecutionStateTracker`, Go `dap-agent` MCP server (NDJSON → `dap-cli` repl) |
| 5 | Navigate, evaluate/scopes/variables, rendered CLI output, `startDebugging` child-session ack |

## Reference

[Meta Dapper](https://github.com/facebookexperimental/dapper) (`reference/dapper/`) informs:

- `MessageRemapper` → `protocol-mux`
- `backend.rs` spawn → `protocol-gateway::SpawnSpec`
- `session_init` → `dap-core` (Phase 1)
- MCP tools → `dap-agent` (Phase 5)
