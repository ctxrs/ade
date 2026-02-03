use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, ValueEnum};

use crate::translate::ReasoningMode;

#[derive(Parser, Debug)]
#[command(name = "acp-crp-bridge")]
#[command(about = "Generic ACP -> CRP bridge", long_about = None)]
#[command(version)]
pub struct Cli {
    /// ACP harness command (quoted string)
    #[arg(long, env = "ACP_COMMAND")]
    pub acp_command: String,

    /// Working directory for the ACP harness
    #[arg(long, env = "ACP_CWD")]
    pub acp_cwd: Option<PathBuf>,

    /// ACP harness env vars (KEY=VALUE)
    #[arg(long = "acp-env", value_parser = parse_key_val, env = "ACP_ENV")]
    pub acp_env: Vec<(String, String)>,

    /// Reasoning handling strategy
    #[arg(long, default_value = "raw-thoughts")]
    pub reasoning_mode: ReasoningModeCli,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub reasoning_mode: ReasoningMode,
}

impl Config {
    pub fn from_cli(cli: Cli) -> Result<Self> {
        let parts = shell_words::split(&cli.acp_command)
            .with_context(|| format!("failed to parse acp-command: {}", cli.acp_command))?;
        let (command, args) = match parts.split_first() {
            Some((cmd, rest)) => (cmd.to_string(), rest.to_vec()),
            None => return Err(anyhow!("acp-command must not be empty")),
        };

        let mut env = HashMap::new();
        for (k, v) in cli.acp_env {
            env.insert(k, v);
        }

        Ok(Self {
            command,
            args,
            cwd: cli.acp_cwd,
            env,
            reasoning_mode: cli.reasoning_mode.into(),
        })
    }
}

#[derive(Clone, Debug, ValueEnum)]
pub enum ReasoningModeCli {
    RawThoughts,
    Omit,
    SummaryIfAvailable,
}

impl From<ReasoningModeCli> for ReasoningMode {
    fn from(value: ReasoningModeCli) -> Self {
        match value {
            ReasoningModeCli::RawThoughts => ReasoningMode::RawThoughts,
            ReasoningModeCli::Omit => ReasoningMode::Omit,
            ReasoningModeCli::SummaryIfAvailable => ReasoningMode::SummaryIfAvailable,
        }
    }
}

fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let mut split = s.splitn(2, '=');
    let key = split
        .next()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "missing env var key".to_string())?;
    let value = split.next().unwrap_or("");
    Ok((key.to_string(), value.to_string()))
}
