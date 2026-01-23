mod metrics;
mod output;
mod scenario;
mod workload;
mod ws;

use anyhow::{anyhow, Result};
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = scenario::Cli::parse();
    let mut scenario = scenario::load_scenario(cli.scenario.as_ref())?;
    scenario::apply_overrides(&mut scenario, &cli);

    match scenario.mode {
        scenario::ScenarioMode::Daemon => workload::run_daemon_mode(&cli, &scenario).await,
        scenario::ScenarioMode::ClientReplay => Err(anyhow!(
            "client_replay mode not implemented yet; use daemon mode first"
        )),
        scenario::ScenarioMode::EndToEnd => workload::run_end_to_end_mode(&cli, &scenario).await,
    }
}
