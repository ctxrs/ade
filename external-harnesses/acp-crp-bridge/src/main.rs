use anyhow::Result;
use clap::Parser;

use acp_crp_bridge::{run, Cli, Config};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::from_cli(cli)?;
    run(config).await
}
