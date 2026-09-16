use anyhow::{Result, bail};
use dap_gdb_remote::config::ConnectConfig;
use dap_plugin_api::AdapterSpawn;
use dap_protocol::DuplexChannel;
use tracing::debug;

use crate::proxy::Backend;

pub async fn spawn_builtin(adapter: &AdapterSpawn) -> Result<Backend> {
    match adapter.command.as_str() {
        "gdb-remote" => spawn_gdb_remote().await,
        other => bail!("unknown builtin adapter `{other}`"),
    }
}

async fn spawn_gdb_remote() -> Result<Backend> {
    debug!("starting builtin gdb-remote adapter");
    let (client_io, adapter_io) = tokio::io::duplex(2 * 1024 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (adapter_read, adapter_write) = tokio::io::split(adapter_io);
    let defaults = ConnectConfig::default();

    tokio::spawn(async move {
        if let Err(err) = dap_gdb_remote::bridge::run(adapter_read, adapter_write, defaults).await {
            tracing::error!("builtin gdb-remote bridge exited: {err:#}");
        }
    });

    Ok(Backend::from_duplex(DuplexChannel::from_streams(client_read, client_write)))
}
