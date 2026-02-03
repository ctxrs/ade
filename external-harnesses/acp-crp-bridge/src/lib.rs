#![deny(clippy::print_stdout, clippy::print_stderr)]

mod bridge;
mod config;
mod crp;
mod translate;

pub use config::{Cli, Config};
pub use translate::ReasoningMode;

pub async fn run(config: Config) -> anyhow::Result<()> {
    bridge::run_bridge(config).await
}
