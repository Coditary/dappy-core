# DAP

Debug Adapter Protocol gateway — split into three standalone projects that can later live in separate repositories.

## Projects

| Directory | Role | Open in Cursor |
|-----------|------|----------------|
| [`dap-core/`](dap-core/) | Backend libraries + `dap-proxy` binary | When working on sessions, routing, mux |
| [`dap-cli/`](dap-cli/) | CLI / REPL control plane | When working on commands, REPL, agent |
| [`dap-plugins/`](dap-plugins/) | Adapter manifests (`builtin/<id>/plugin.yaml`) | When adding or editing adapters |

The gdb-remote adapter lives in a separate repo: [coditary/dap-gdb-remote](https://github.com/coditary/dap-gdb-remote) (pulled in by `dap-core` as a git dependency).

## Quick start

```bash
# Backend
cd dap-core && cargo build && cargo test --workspace

# CLI (depends on dap-core via path; loads manifests from dap-plugins/builtin)
cd dap-cli && cargo build && cargo test

# Editor proxy
./dap-core/target/debug/dap-proxy --program main.py

# Interactive debugger
./dap-cli/target/debug/dap-cli debug repl --adapter fake main.py
```

## Multi-root workspace

Open [`dap.code-workspace`](dap.code-workspace) in Cursor/VS Code to work on all three projects side by side without indexing everything as one flat tree.

## Splitting into separate repos

1. Move each directory to its own git repository.
2. In `dap-cli/Cargo.toml`, replace `path = "../dap-core/..."` with git tags or crates.io versions.
3. Ship `dap-plugins/builtin` alongside binaries or set `DAP_PLUGINS_DIR`.
