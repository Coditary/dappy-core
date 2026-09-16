use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::error::InstanceError;
use crate::handle::InstanceHandle;
use crate::id::InstanceId;
use crate::spec::InstanceSpec;
use crate::state::InstanceState;
use crate::store::{SessionRecord, SessionStore};

/// In-memory registry for multiple concurrent instances.
///
/// Thread-safe and async-friendly. Supports process supervision and optional
/// synchronization with the on-disk [`SessionStore`].
#[derive(Debug, Clone, Default)]
pub struct InstanceManager {
    inner: Arc<RwLock<HashMap<InstanceId, InstanceHandle>>>,
}

impl InstanceManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new instance in `Pending` state.
    pub async fn register(&self, spec: InstanceSpec) -> Result<InstanceId, InstanceError> {
        let id = spec.id.clone();
        let kind = spec.kind.clone();
        let handle = InstanceHandle {
            spec,
            state: InstanceState::Pending,
        };

        {
            let mut map = self.inner.write().await;
            if map.contains_key(&id) {
                return Err(InstanceError::AlreadyExists(id.to_string()));
            }
            map.insert(id.clone(), handle);
        }

        info!(instance_id = %id, kind = %kind, "registered instance");
        Ok(id)
    }

    /// List all instances, optionally filtered by scope.
    pub async fn list(&self, scope: Option<&str>) -> Vec<InstanceHandle> {
        let map = self.inner.read().await;
        map.values()
            .filter(|h| match scope {
                Some(s) => h.spec.scope.as_deref() == Some(s),
                None => true,
            })
            .cloned()
            .collect()
    }

    pub async fn get(&self, id: &InstanceId) -> Result<InstanceHandle, InstanceError> {
        let map = self.inner.read().await;
        map.get(id)
            .cloned()
            .ok_or_else(|| InstanceError::NotFound(id.to_string()))
    }

    pub async fn set_state(
        &self,
        id: &InstanceId,
        next: InstanceState,
    ) -> Result<InstanceHandle, InstanceError> {
        let mut map = self.inner.write().await;
        let handle = map
            .get_mut(id)
            .ok_or_else(|| InstanceError::NotFound(id.to_string()))?;

        if !handle.state.can_transition_to(next) {
            return Err(InstanceError::InvalidTransition {
                from: handle.state.label().to_owned(),
                to: next.label().to_owned(),
            });
        }

        debug!(
            instance_id = %id,
            from = handle.state.label(),
            to = next.label(),
            "instance state transition"
        );
        handle.state = next;
        Ok(handle.clone())
    }

    pub async fn remove(&self, id: &InstanceId) -> Result<InstanceHandle, InstanceError> {
        let mut map = self.inner.write().await;
        map.remove(id)
            .ok_or_else(|| InstanceError::NotFound(id.to_string()))
    }

    pub async fn count(&self) -> usize {
        self.inner.read().await.len()
    }

    pub async fn set_control_port(
        &self,
        id: &InstanceId,
        port: u16,
    ) -> Result<InstanceHandle, InstanceError> {
        let mut map = self.inner.write().await;
        let handle = map
            .get_mut(id)
            .ok_or_else(|| InstanceError::NotFound(id.to_string()))?;
        handle.spec.control_port = Some(port);
        Ok(handle.clone())
    }

    pub async fn set_pid(
        &self,
        id: &InstanceId,
        pid: u32,
    ) -> Result<InstanceHandle, InstanceError> {
        let mut map = self.inner.write().await;
        let handle = map
            .get_mut(id)
            .ok_or_else(|| InstanceError::NotFound(id.to_string()))?;
        handle.spec.pid = Some(pid);
        Ok(handle.clone())
    }

    /// Remove instances whose owning process or control port is no longer alive.
    pub async fn prune_stale(&self) -> Vec<InstanceId> {
        let stale = self
            .list(None)
            .await
            .into_iter()
            .filter(|handle| !handle.is_alive())
            .map(|handle| handle.id().clone())
            .collect::<Vec<_>>();

        let mut removed = Vec::new();
        for id in stale {
            if self.remove(&id).await.is_ok() {
                removed.push(id);
            }
        }
        removed
    }

    /// Import active sessions from the on-disk store into this manager.
    pub async fn import_from_store(
        &self,
        store: &SessionStore,
    ) -> Result<Vec<InstanceId>, InstanceError> {
        let records = store.list_active()?;
        let mut imported = Vec::new();

        for record in records {
            let id = InstanceId::new(record.instance_id.clone());
            if self.get(&id).await.is_ok() {
                continue;
            }

            let mut spec = InstanceSpec::new("dap-session")
                .with_id(id.clone())
                .with_control_port(record.control_port)
                .with_pid(record.pid)
                .with_tag("adapter", record.adapter_id);
            if let Some(program) = record.program {
                spec = spec.with_label(program);
            }
            if let Some(scope) = record.scope {
                spec = spec.with_scope(scope);
            }
            if let Some(parent_id) = record.parent_id {
                spec = spec.with_parent_id(InstanceId::new(parent_id));
            }

            self.register(spec).await?;
            self.set_state(&id, InstanceState::Running).await?;
            imported.push(id);
        }

        Ok(imported)
    }

    /// Persist a running instance to the on-disk session store.
    pub fn persist_to_store(
        handle: &InstanceHandle,
        store: &SessionStore,
    ) -> Result<(), InstanceError> {
        let Some(record) = session_record_from_handle(handle) else {
            return Ok(());
        };
        store.save(&record)
    }
}

/// Build a cross-process session record from a managed instance handle.
pub fn session_record_from_handle(handle: &InstanceHandle) -> Option<SessionRecord> {
    let control_port = handle.spec.control_port?;
    let pid = handle.spec.pid?;
    let adapter_id = handle.spec.tags.get("adapter")?.clone();

    Some(SessionRecord {
        instance_id: handle.id().to_string(),
        pid,
        control_port,
        adapter_id,
        program: handle.spec.label.clone(),
        scope: handle.spec.scope.clone(),
        parent_id: handle
            .spec
            .parent_id
            .as_ref()
            .map(|parent| parent.to_string()),
        started_at_unix: handle.spec.created_at_unix,
    })
}
