use anyhow::{Context, Result};
use clap::Parser;
use dap_core::ControlClient;
use instance_manager::SessionStore;
use serde_json::json;
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};

use crate::commands::GlobalOpts;
use crate::output::print_json;

#[derive(Parser)]
pub struct Session {
    #[command(subcommand)]
    command: SessionCommands,
}

#[derive(clap::Subcommand)]
enum SessionCommands {
    /// List active debug sessions (from on-disk session store)
    List,
    /// Show one session record and reachability probes
    Show {
        /// Session instance id
        instance_id: String,
    },
    /// End a debug session via the control plane
    Kill {
        /// Session instance id
        instance_id: String,
        /// Also terminate the debuggee process
        #[arg(long)]
        terminate: bool,
    },
}

impl Session {
    pub async fn run(self, globals: GlobalOpts) -> Result<()> {
        match self.command {
            SessionCommands::List => {
                let store = SessionStore::open(SessionStore::default_dir())?;
                let sessions = store.list_active()?;
                let filtered = filter_by_scope(sessions, &globals.scope);
                print_json(&json!(filtered), globals.json);
            }
            SessionCommands::Show { instance_id } => {
                let store = SessionStore::open(SessionStore::default_dir())?;
                let record = store
                    .get(&instance_id)?
                    .with_context(|| format!("session not found or stale: {instance_id}"))?;
                let control_reachable = probe_control_port(record.control_port).await;
                print_json(
                    &json!({
                        "record": record,
                        "alive": record.is_alive(),
                        "control_reachable": control_reachable,
                    }),
                    globals.json,
                );
            }
            SessionCommands::Kill {
                instance_id,
                terminate,
            } => {
                let store = SessionStore::open(SessionStore::default_dir())?;
                let record = store
                    .get(&instance_id)?
                    .with_context(|| format!("session not found or stale: {instance_id}"))?;
                let mut client = ControlClient::connect(record.control_port).await?;
                if terminate {
                    client.terminate().await?;
                } else {
                    client.disconnect().await?;
                }
                let _ = store.delete(&instance_id);
                print_json(
                    &json!({
                        "instance_id": instance_id,
                        "disconnected": true,
                        "terminate_debuggee": terminate,
                    }),
                    globals.json,
                );
            }
        }
        Ok(())
    }
}

fn filter_by_scope(
    sessions: Vec<instance_manager::SessionRecord>,
    scope: &Option<String>,
) -> Vec<instance_manager::SessionRecord> {
    if let Some(scope) = scope {
        sessions
            .into_iter()
            .filter(|s| s.scope.as_deref() == Some(scope.as_str()))
            .collect()
    } else {
        sessions
    }
}

async fn probe_control_port(port: u16) -> bool {
    timeout(Duration::from_secs(2), TcpStream::connect(("127.0.0.1", port)))
        .await
        .ok()
        .and_then(|result| result.ok())
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use instance_manager::SessionRecord;

    #[test]
    fn filter_by_scope_keeps_matching_sessions() {
        let sessions = vec![
            SessionRecord {
                instance_id: "a".into(),
                pid: 1,
                control_port: 1,
                adapter_id: "fake".into(),
                program: None,
                scope: Some("ws".into()),
                parent_id: None,
                started_at_unix: 0,
            },
            SessionRecord {
                instance_id: "b".into(),
                pid: 1,
                control_port: 2,
                adapter_id: "fake".into(),
                program: None,
                scope: None,
                parent_id: None,
                started_at_unix: 0,
            },
        ];
        let filtered = filter_by_scope(sessions, &Some("ws".into()));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].instance_id, "a");
    }

    #[tokio::test]
    async fn probe_control_port_false_for_closed_port() {
        assert!(!probe_control_port(9).await);
    }
}
