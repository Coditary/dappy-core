# Development

## Formatting

```bash
cargo fmt --all
```

## Lint

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

## Tests

```bash
cargo test --workspace
```

Integration test with fake DAP adapter:

```bash
cargo test -p fake-dap-adapter --test initialize_e2e
```

Multiplex integration tests (Phase 2):

```bash
cargo test -p protocol-mux --test multiplex_e2e
cargo test -p dap-core --test mux_integration
cargo test -p dap-proxy --test control_attach_e2e
```

Phase 3 (session store, control client, routing):

```bash
cargo test -p instance-manager --test store
cargo test -p dap-core --test control_client
```

Phase 4 (session init, execution state, MCP agent):

```bash
cargo test -p dap-core --test session_init
go test ./dap-agent/...
go build -o target/debug/dap-agent ./dap-agent
```

Phase 5 (navigation, evaluate, reverse requests, rendered output):

```bash
cargo test -p dap-core --test navigation
cargo test -p dap-core --test reverse_request
cargo test -p dap-core --test control_client
```

Rust plugin routing:

```bash
cargo test -p dap-core --test engine
cargo test -p dap-core rust_routing
```

Stdio proxy integration tests (Phase 1):

```bash
cargo test -p dap-proxy --test stdio_proxy_e2e
```

## CI

GitHub Actions workflow: [`.github/workflows/ci.yml`](../.github/workflows/ci.yml)
