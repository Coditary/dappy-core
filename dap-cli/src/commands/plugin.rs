use anyhow::{Context, Result};
use clap::Parser;
use dap_core::DapEngine;

use crate::commands::GlobalOpts;
use crate::output::print_json;

#[derive(Parser)]
pub struct Plugin {
    #[command(subcommand)]
    command: PluginCommands,
}

#[derive(clap::Subcommand)]
enum PluginCommands {
    /// List registered plugins
    List,
    /// Show one plugin (includes resolved attachment path when configured)
    Show {
        /// Plugin id (e.g. python, rust)
        id: String,
    },
}

impl Plugin {
    pub async fn run(self, _globals: GlobalOpts) -> Result<()> {
        match self.command {
            PluginCommands::List => {
                let engine = DapEngine::with_builtin_plugins()?;
                let plugins = engine
                    .registry
                    .list()
                    .iter()
                    .map(plugin_summary_json)
                    .collect::<Vec<_>>();
                print_json(&serde_json::json!(plugins), false);
            }
            PluginCommands::Show { id } => {
                let engine = DapEngine::with_builtin_plugins()?;
                let manifest = engine
                    .registry
                    .get(&id)
                    .with_context(|| format!("plugin not found: {id}"))?;
                print_json(&plugin_detail_json(manifest)?, false);
            }
        }
        Ok(())
    }
}

fn plugin_summary_json(manifest: &dap_plugin_api::PluginManifest) -> serde_json::Value {
    let mut value = serde_json::json!({
        "id": manifest.id,
        "name": manifest.name,
        "version": manifest.version,
        "launchTypes": manifest.launch_types,
        "fileExtensions": manifest.file_extensions,
    });
    if let Some(path) = manifest.attachment_path() {
        value["attachment"] = serde_json::json!(path.display().to_string());
    }
    value
}

fn plugin_detail_json(manifest: &dap_plugin_api::PluginManifest) -> Result<serde_json::Value> {
    let mut value = plugin_summary_json(manifest);
    if let Some(spec) = &manifest.attachment {
        value["attachmentSpec"] = serde_json::json!(spec);
    }
    if let Some(path) = manifest.attachment_path() {
        let attachment = dap_plugin_api::load_attachment(&path)
            .with_context(|| format!("load attachment {}", path.display()))?;
        value["attachmentBody"] = serde_json::to_value(attachment)
            .context("serialize attachment body as JSON")?;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::GlobalOpts;

    #[tokio::test]
    async fn plugin_list_renders_builtin_plugins() {
        let cmd = Plugin {
            command: PluginCommands::List,
        };
        cmd.run(GlobalOpts {
            json: false,
            control_port: None,
            scope: None,
        })
        .await
        .expect("plugin list");
    }

    #[tokio::test]
    async fn plugin_show_resolves_python_attachment() {
        let cmd = Plugin {
            command: PluginCommands::Show {
                id: "python".into(),
            },
        };
        cmd.run(GlobalOpts {
            json: true,
            control_port: None,
            scope: None,
        })
        .await
        .expect("plugin show");
    }
}
