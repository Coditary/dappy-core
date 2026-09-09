use anyhow::Result;
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
                    .map(|p| {
                        serde_json::json!({
                            "id": p.id,
                            "name": p.name,
                            "version": p.version,
                            "launchTypes": p.launch_types,
                            "fileExtensions": p.file_extensions,
                        })
                    })
                    .collect::<Vec<_>>();
                print_json(&serde_json::json!(plugins), false);
            }
        }
        Ok(())
    }
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
}
