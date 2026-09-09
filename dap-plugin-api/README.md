# dap-plugin-api

Adapter plugin manifest types, validation, and YAML loading. No runtime dependency on `dap-core`.

```toml
dap-plugin-api = { path = "../dap-plugin-api" }
```

```rust
use dap_plugin_api::{load_from_dir, default_builtin_dir};

let plugins = load_from_dir(&default_builtin_dir())?;
```

Plugin manifests live in `plugins/builtin/*.yaml` (or `$DAP_PLUGINS_DIR`). User overrides can be placed in `$XDG_CONFIG_HOME/dap/plugins`.
