use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use dap_core::{ChildBackendPlan, ChildSessionSpawner, ChildSpawnPlan, ChildSpawnResult};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tracing::warn;

/// Tracks spawned child proxy processes for teardown.
pub struct ChildProcessSupervisor {
    exe: PathBuf,
    scope: Option<String>,
    parent_instance_id: String,
    child_profile: String,
    child_profile_file: Option<PathBuf>,
    child_max_children: u32,
    children: Mutex<Vec<Child>>,
}

impl ChildProcessSupervisor {
    pub fn new(
        parent_instance_id: String,
        scope: Option<String>,
        child_profile: String,
        child_profile_file: Option<PathBuf>,
        child_max_children: u32,
    ) -> Result<Self> {
        Ok(Self {
            exe: std::env::current_exe().context("resolve dap-proxy executable")?,
            scope,
            parent_instance_id,
            child_profile,
            child_profile_file,
            child_max_children,
            children: Mutex::new(Vec::new()),
        })
    }

    pub async fn teardown(&self) {
        let mut children = self.children.lock().await;
        for child in children.iter_mut() {
            let _ = child.start_kill();
        }
        for child in children.iter_mut() {
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
        }
        children.clear();
    }
}

pub struct ProxyChildSpawner {
    supervisor: Arc<ChildProcessSupervisor>,
}

impl ProxyChildSpawner {
    pub fn new(supervisor: Arc<ChildProcessSupervisor>) -> Self {
        Self { supervisor }
    }
}

#[async_trait::async_trait]
impl ChildSessionSpawner for ProxyChildSpawner {
    async fn spawn(&self, plan: ChildSpawnPlan) -> Result<ChildSpawnResult> {
        let mut cmd = Command::new(&self.supervisor.exe);
        cmd.arg("--headless")
            .arg("--control-port")
            .arg("0")
            .arg("--parent-id")
            .arg(&self.supervisor.parent_instance_id)
            .arg("--child-profile")
            .arg(&self.supervisor.child_profile);
        if let Some(path) = &self.supervisor.child_profile_file {
            cmd.arg("--child-profile-file").arg(path);
        }
        if plan.child_depth > 0 {
            cmd.arg("--child-sessions");
        }
        cmd.arg("--child-depth")
            .arg(plan.child_depth.to_string())
            .arg("--child-max-children")
            .arg(self.supervisor.child_max_children.to_string())
            .arg("--debug-request")
            .arg(&plan.debug_request)
            .arg("--debug-args")
            .arg(plan.debug_arguments.to_string());
        match &plan.backend {
            ChildBackendPlan::Stdio { command, args } => {
                cmd.arg("--adapter-cmd").arg(command);
                for arg in args {
                    cmd.arg(arg);
                }
            }
            ChildBackendPlan::Tcp { host, port } => {
                cmd.arg("--adapter-tcp").arg(format!("{host}:{port}"));
            }
        }
        if let Some(scope) = &self.supervisor.scope {
            cmd.arg("--scope").arg(scope);
        }
        if let Ok(dir) = std::env::var("DAP_SESSIONS_DIR") {
            cmd.env("DAP_SESSIONS_DIR", dir);
        }

        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().context("spawn child dap-proxy")?;
        let stderr = child.stderr.take().context("child stderr")?;
        let ready = read_child_ready(stderr).await?;
        self.supervisor.children.lock().await.push(child);
        Ok(ready)
    }
}

async fn read_child_ready(stderr: impl tokio::io::AsyncRead + Unpin) -> Result<ChildSpawnResult> {
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline {
        line.clear();
        let read = tokio::time::timeout(
            deadline.saturating_duration_since(tokio::time::Instant::now()),
            reader.read_line(&mut line),
        )
        .await
        .context("timed out waiting for child ready line")??;
        if read == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let (Some(port), Some(instance_id)) = (
                json.get("controlPort").and_then(|v| v.as_u64()),
                json.get("instanceId").and_then(|v| v.as_str()),
            ) {
                return Ok(ChildSpawnResult {
                    control_port: port as u16,
                    instance_id: instance_id.to_string(),
                });
            }
        }
        warn!(line = trimmed, "unexpected child proxy stderr line");
    }
    anyhow::bail!("child proxy did not report controlPort/instanceId")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn supervisor_tracks_executable_path() {
        let supervisor = ChildProcessSupervisor::new(
            "parent".into(),
            None,
            "fake".into(),
            None,
            16,
        )
        .expect("supervisor");
        assert!(supervisor.exe.exists() || supervisor.exe.file_name().is_some());
    }

    #[tokio::test]
    async fn teardown_kills_tracked_children() {
        let supervisor = ChildProcessSupervisor::new(
            "parent".into(),
            None,
            "fake".into(),
            None,
            16,
        )
        .expect("supervisor");
        let mut child = Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        supervisor.children.lock().await.push(child);
        supervisor.teardown().await;
        std::thread::sleep(Duration::from_millis(100));
        if let Some(pid) = pid {
            assert!(!is_pid_alive(pid));
        }
    }

    fn is_pid_alive(pid: u32) -> bool {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
}
