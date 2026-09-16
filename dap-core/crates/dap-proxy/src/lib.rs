//! Library surface for integration tests and shared proxy logic.

use anyhow::{Context, Result};
use dap_core::{DapEngine, LaunchOptions};
use dap_plugin_api::AdapterSpawn;
use instance_manager::SessionStore;

pub const BIN_NAME: &str = "dap-proxy";

pub mod child_supervisor;
pub use child_supervisor::{ChildProcessSupervisor, ProxyChildSpawner};

/// Deletes the on-disk session record when the proxy exits for any reason.
pub struct SessionCleanupGuard {
    store: SessionStore,
    instance_id: Option<String>,
}

impl SessionCleanupGuard {
    pub fn new(store: SessionStore) -> Self {
        // Prune stale session files as a side effect of listing.
        let _ = store.list_active();
        Self {
            store,
            instance_id: None,
        }
    }

    pub fn arm(&mut self, instance_id: String) {
        self.instance_id = Some(instance_id);
    }

    pub fn save(&self, record: &instance_manager::SessionRecord) -> Result<()> {
        self.store.save(record).map_err(Into::into)
    }

    pub fn disarm(&mut self) {
        self.instance_id = None;
    }
}

impl Drop for SessionCleanupGuard {
    fn drop(&mut self) {
        if let Some(instance_id) = &self.instance_id {
            if let Err(err) = self.store.delete(instance_id) {
                tracing::warn!("failed to delete session file for {instance_id}: {err}");
            }
        }
    }
}

pub fn resolve_adapter_spawn(
    adapter_cmd: Option<&[String]>,
    engine: &DapEngine,
    launch: &LaunchOptions,
) -> Result<AdapterSpawn> {
    if let Some(cmd) = adapter_cmd {
        let mut parts = cmd.to_vec();
        let command = parts
            .first()
            .context("--adapter-cmd requires a command")?
            .clone();
        parts.remove(0);
        return Ok(AdapterSpawn {
            transport: dap_plugin_api::SpawnTransport::Stdio,
            command,
            args: parts,
        });
    }

    engine
        .resolve_adapter_spawn(launch)
        .with_context(|| format!("resolve adapter for launch {:?}", launch.program))
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use instance_manager::SessionRecord;

    #[test]
    fn cleanup_guard_deletes_armed_session() {
        let dir = std::env::temp_dir().join(format!(
            "dap-proxy-guard-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("tmpdir");
        let store = SessionStore::open(&dir).expect("open store");
        let record = SessionRecord {
            instance_id: "test-instance".into(),
            pid: 1,
            control_port: 9,
            adapter_id: "fake".into(),
            program: Some("main.py".into()),
            scope: None,
            parent_id: None,
            started_at_unix: now_unix(),
        };
        store.save(&record).expect("save");

        let mut guard = SessionCleanupGuard::new(store);
        guard.arm("test-instance".into());
        drop(guard);

        let store = SessionStore::open(&dir).expect("reopen");
        assert!(store.get("test-instance").expect("get").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_adapter_spawn_prefers_explicit_cmd() {
        let mut engine = DapEngine::new();
        engine.load_builtin_plugins().expect("plugins");
        let launch = LaunchOptions {
            request: "launch".into(),
            program: Some("main.py".into()),
            adapter: None,
            extra: serde_json::json!({}),
        };
        let spawn = resolve_adapter_spawn(
            Some(&["/bin/echo".into(), "hello".into()]),
            &engine,
            &launch,
        )
        .expect("spawn");
        assert_eq!(spawn.command, "/bin/echo");
        assert_eq!(spawn.args, vec!["hello".to_string()]);
    }

    #[test]
    fn now_unix_is_positive() {
        assert!(now_unix() > 0);
    }
}
