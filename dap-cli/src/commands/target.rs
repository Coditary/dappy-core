use anyhow::{Context, Result};
use clap::Parser;
use dap_core::DapEngine;

use crate::commands::GlobalOpts;
use crate::output::print_json;

#[derive(Parser)]
pub struct Target {
    #[command(subcommand)]
    command: TargetCommands,
}

#[derive(clap::Subcommand)]
enum TargetCommands {
    /// List registered RSP target manifests
    List,
    /// Show one target manifest
    Show {
        /// Target id (e.g. gdbserver, qemu-x86-kernel)
        id: String,
    },
}

impl Target {
    pub async fn run(self, _globals: GlobalOpts) -> Result<()> {
        match self.command {
            TargetCommands::List => {
                let engine = DapEngine::with_builtin_plugins()?;
                let targets = engine
                    .target_registry
                    .list()
                    .iter()
                    .map(target_summary_json)
                    .collect::<Vec<_>>();
                print_json(&serde_json::json!(targets), false);
            }
            TargetCommands::Show { id } => {
                let engine = DapEngine::with_builtin_plugins()?;
                let manifest = engine
                    .target_registry
                    .get(&id)
                    .with_context(|| format!("target not found: {id}"))?;
                print_json(&target_detail_json(manifest)?, false);
            }
        }
        Ok(())
    }
}

fn target_summary_json(manifest: &dap_plugin_api::TargetManifest) -> serde_json::Value {
    serde_json::json!({
        "id": manifest.id,
        "name": manifest.name,
        "version": manifest.version,
        "targetTypes": manifest.target_types,
        "description": manifest.description,
        "spawn": manifest.spawn.as_ref().map(|spawn| spawn.command.clone()),
    })
}

fn target_detail_json(manifest: &dap_plugin_api::TargetManifest) -> Result<serde_json::Value> {
    let mut value = target_summary_json(manifest);
    if let Some(path) = manifest.source_path.as_ref() {
        value["source"] = serde_json::json!(path.display().to_string());
    }
    value["connect"] = serde_json::to_value(manifest.connect.clone().unwrap_or_default())
        .context("serialize connect block")?;
    value["attach"] = manifest.attach.clone().unwrap_or(serde_json::json!({}));
    if let Some(spawn) = manifest.spawn.as_ref() {
        value["spawn"] = serde_json::to_value(spawn).context("serialize spawn block")?;
    }
    Ok(value)
}
