use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::InstanceError;

/// Persisted record for a running multiplexed DAP session (cross-process discovery).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRecord {
    pub instance_id: String,
    pub pid: u32,
    pub control_port: u16,
    pub adapter_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub started_at_unix: i64,
}

impl SessionRecord {
    /// Whether the owning process is alive and the control port is still bound.
    pub fn is_alive(&self) -> bool {
        is_pid_alive(self.pid) && is_control_port_in_use(self.control_port)
    }
}

/// File-backed session registry for CLI / agent discovery.
pub struct SessionStore {
    dir: PathBuf,
}

impl SessionStore {
    /// Default directory: `$DAP_SESSIONS_DIR` or `$XDG_DATA_HOME/dap/sessions`.
    pub fn default_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("DAP_SESSIONS_DIR") {
            return PathBuf::from(dir);
        }
        let base = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|_| std::env::var("HOME").map(|home| PathBuf::from(home).join(".local/share")))
            .unwrap_or_else(|_| PathBuf::from("/tmp"));
        base.join("dap/sessions")
    }

    pub fn open(dir: impl AsRef<Path>) -> Result<Self, InstanceError> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)
            .map_err(|err| InstanceError::Other(format!("create sessions dir: {err}")))?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn save(&self, record: &SessionRecord) -> Result<(), InstanceError> {
        let path = self.record_path(&record.instance_id);
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(record)
            .map_err(|err| InstanceError::Other(format!("serialize session: {err}")))?;
        fs::write(&tmp, body)
            .map_err(|err| InstanceError::Other(format!("write session tmp: {err}")))?;
        fs::rename(&tmp, &path)
            .map_err(|err| InstanceError::Other(format!("commit session file: {err}")))?;
        Ok(())
    }

    pub fn delete(&self, instance_id: &str) -> Result<(), InstanceError> {
        let path = self.record_path(instance_id);
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|err| InstanceError::Other(format!("delete session file: {err}")))?;
        }
        Ok(())
    }

    pub fn get(&self, instance_id: &str) -> Result<Option<SessionRecord>, InstanceError> {
        let path = self.record_path(instance_id);
        if !path.exists() {
            return Ok(None);
        }
        let record = self.read_record(&path)?;
        if record.is_alive() {
            Ok(Some(record))
        } else {
            let _ = fs::remove_file(&path);
            Ok(None)
        }
    }

    /// List sessions whose owning process is still alive (stale files are removed).
    pub fn list_active(&self) -> Result<Vec<SessionRecord>, InstanceError> {
        let mut records = Vec::new();
        let entries = fs::read_dir(&self.dir)
            .map_err(|err| InstanceError::Other(format!("read sessions dir: {err}")))?;
        for entry in entries {
            let path = entry
                .map_err(|err| InstanceError::Other(format!("read dir entry: {err}")))?
                .path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let record = match self.read_record(&path) {
                    Ok(record) => record,
                    Err(_) => continue,
                };
                if record.is_alive() {
                    records.push(record);
                } else {
                    let _ = fs::remove_file(&path);
                }
            }
        }
        records.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
        Ok(records)
    }

    fn record_path(&self, instance_id: &str) -> PathBuf {
        self.dir.join(format!("{instance_id}.json"))
    }

    fn read_record(&self, path: &Path) -> Result<SessionRecord, InstanceError> {
        let body = fs::read_to_string(path)
            .map_err(|err| InstanceError::Other(format!("read session {:?}: {err}", path)))?;
        match serde_json::from_str(&body) {
            Ok(record) => Ok(record),
            Err(err) => {
                let _ = fs::remove_file(path);
                Err(InstanceError::Other(format!(
                    "parse session {:?}: {err}",
                    path
                )))
            }
        }
    }
}

/// Returns true when a TCP listener accepts connections on the control attach port.
pub fn is_control_port_in_use(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok()
}

pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        use std::process::Command;
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}
