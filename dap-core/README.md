# dap-core

Backend workspace: DAP proxy libraries and the `dap-proxy` editor binary.

## Crates

| Crate | Role |
|-------|------|
| [`crates/dap-core`](crates/dap-core/) | Sessions, router, engine |
| [`crates/dap-protocol`](crates/dap-protocol/) | DAP messages and framing |
| [`crates/dap-plugin-api`](crates/dap-plugin-api/) | Manifest loader (reads `dap-plugins/builtin`) |
| [`crates/instance-manager`](crates/instance-manager/) | Generic instance registry |
| [`crates/protocol-gateway`](crates/protocol-gateway/) | Plugin registry + routing |
| [`crates/protocol-mux`](crates/protocol-mux/) | N clients → 1 upstream |
| [`crates/dap-proxy`](crates/dap-proxy/) | Editor-facing stdio binary |

Plugin manifests live in the sibling [`../dap-plugins`](../dap-plugins/) project.

## Build & test

```bash
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

See [docs/architecture.md](docs/architecture.md) for data flow.
