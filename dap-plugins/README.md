# dap-plugins

Builtin manifests for the DAP proxy. **YAML only** — no code, no build step.

Runtime loading and validation live in [`dap-plugin-api`](../dap-core/crates/dap-plugin-api/) (`dap-core` workspace).

## Two manifest kinds (symmetric)

| Kind | File | Describes |
|------|------|-----------|
| **DAP adapter** | `plugin.yaml` | External debug adapter (debugpy, lldb-dap, …) |
| **RSP target** | `target.yaml` | gdb-remote backend (QEMU, gdbserver, attach-only) |

A directory has **either** `plugin.yaml` **or** `target.yaml` — not both for the same role.

```
builtin/
  python/plugin.yaml          # DAP
  rust/plugin.yaml            # DAP
  gdb-remote/plugin.yaml      # DAP bridge (until builtin in proxy)
  gdbserver/target.yaml       # RSP
  qemu-x86-kernel/target.yaml # RSP
  rsp-attach/target.yaml      # RSP
```

## Plugin manifest (`plugin.yaml`)

| Section | Purpose |
|---------|---------|
| `adapter` | Spawn command (`stdio` or `tcp`) |
| `initialize` | Headless `initialize` payload |
| `launch` | Headless `launch` / `attach` payload |
| `init.steps` | Ordered post-`initialize` DAP sequence |
| `attachment` | Optional client-side companion YAML |

```bash
dap-cli plugin list
dap-cli plugin show python
```

## Target manifest (`target.yaml`)

Parallel schema for RSP backends. The proxy/cli spawns the target, then attaches via the builtin gdb-remote bridge.

| Section | Purpose |
|---------|---------|
| `targetTypes` | Routing tags (`gdb-remote`, `qemu`, `rsp`) |
| `spawn` | Start QEMU / gdbserver / OpenOCD |
| `connect` | `host` + `port` for gdb-remote |
| `attach` | Fields for gdb-remote attach (`program`, `imageBase`, `decompiler`, …) |
| `init` | Optional pre-attach steps (future) |

```bash
dap-cli target list
dap-cli target show gdbserver
dap-cli debug repl --target gdbserver -- ./app.elf

# Editor / nvim-dap via dap-proxy (spawns gdbserver, builtin gdb-remote adapter)
dap-proxy --stdio --target gdbserver --program ./app.elf
```

Example `target.yaml`:

```yaml
id: gdbserver
name: GDB Server (local)
version: 0.1.0
targetTypes: [gdb-remote, rsp]

spawn:
  command: gdbserver
  args: [":1234", "${program.abs}"]

connect:
  host: 127.0.0.1
  port: 1234

attach:
  program: "${program.abs}"
  decompiler: auto
```

### Templates

Same placeholders as plugins: `${program.abs}`, `${program.parent}`, `${env:VAR}`, …

### Discovery

```bash
export DAP_PLUGINS_DIR=/path/to/builtin   # plugins + targets
```

User overrides:

- Plugins: `$XDG_CONFIG_HOME/dap/plugins/<id>/plugin.yaml`
- Targets: `$XDG_CONFIG_HOME/dap/targets/<id>/target.yaml`

## gdb-remote today

- **RSP targets** → standalone `target.yaml` entries (`gdbserver`, `qemu-x86-kernel`, …)
- **DAP bridge** → builtin in `dap-proxy` / `dap-core` (`adapter.transport: builtin`, `command: gdb-remote`)
- **Standalone** → [`dap-gdb-remote`](https://github.com/coditary/dap-gdb-remote) binary still works for direct stdio use

When `--target` is set, `dap-cli` defaults to adapter `gdb-remote` if `--adapter` is omitted.
