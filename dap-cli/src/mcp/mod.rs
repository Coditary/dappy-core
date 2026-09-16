mod server;

use anyhow::Result;

use crate::commands::GlobalOpts;

#[derive(Debug, Clone)]
pub struct McpOptions {
    pub globals: GlobalOpts,
    pub program: Option<String>,
    pub adapter: Option<String>,
    pub target: Option<String>,
}

pub async fn run(options: McpOptions) -> Result<()> {
    server::run(options).await
}
