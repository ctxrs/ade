use anyhow::Result;
use clap::Parser;

use rovo_dev_acp::{run, Cli, Config};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::from_cli(cli);
    run(config).await
}
