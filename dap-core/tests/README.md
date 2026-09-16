# Test fixtures

| Path | Description |
|------|-------------|
| [`fixtures/fake-dap-adapter/`](fixtures/fake-dap-adapter/) | Minimal stdio DAP server for integration tests |

Build:

```bash
cargo build -p fake-dap-adapter
```

Integration test:

```bash
cargo test -p fake-dap-adapter --test initialize_e2e
```
