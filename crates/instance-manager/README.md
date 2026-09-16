# instance-manager

Protocol-agnostic library for tracking and managing multiple concurrent instances
(debug adapters, language servers, proxies, etc.).

Designed to be reused across this DAP workspace and separate LSP tooling.

## Usage (standalone)

Add to another project's `Cargo.toml`:

```toml
instance-manager = { path = "../dap/instance-manager" }
# or, once published:
# instance-manager = "0.1"
```

```rust
use instance_manager::{InstanceManager, InstanceSpec, InstanceState, SessionStore};

let manager = InstanceManager::new();
let id = manager
    .register(InstanceSpec::new("dap-session").with_label("main.py"))
    .await?;
manager.set_state(&id, InstanceState::Running).await?;

// Cross-process discovery via on-disk session files
let store = SessionStore::open(SessionStore::default_dir())?;
let imported = manager.import_from_store(&store).await?;
let removed = manager.prune_stale().await;
```
