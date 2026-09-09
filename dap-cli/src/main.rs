use anyhow::Result;
use clap::{Parser, Subcommand};
use dap_cli::commands::GlobalOpts;
use tracing::info;

#[derive(Parser)]
#[command(name = "dap-cli", about = "DAP proxy command-line interface")]
struct Cli {
    /// Emit compact JSON (default: pretty-printed).
    #[arg(long, global = true)]
    json: bool,

    /// Control attach port (default: auto-discover sole active session).
    #[arg(long, global = true)]
    control_port: Option<u16>,

    /// Filter sessions by scope id (also `DAP_SCOPE_ID`).
    #[arg(long, global = true, env = "DAP_SCOPE_ID")]
    scope: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Session management
    Session(dap_cli::commands::session::Session),
    /// Plugin / adapter registry
    Plugin(dap_cli::commands::plugin::Plugin),
    /// Start and control debug sessions
    Debug(dap_cli::commands::debug::Debug),
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let cli = Cli::parse();
    let globals = GlobalOpts {
        json: cli.json,
        control_port: cli.control_port,
        scope: cli.scope,
    };

    match cli.command {
        Commands::Session(cmd) => cmd.run(globals).await?,
        Commands::Plugin(cmd) => cmd.run(globals).await?,
        Commands::Debug(cmd) => cmd.run(globals).await?,
    }

    info!("done");
    Ok(())
}
