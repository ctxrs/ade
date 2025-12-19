use clap::Parser;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "rovo-dev-acp", version, about = "ACP stdio server for Rovo Dev CLI")]
pub struct Cli {
    /// Base URL for an existing Rovo Dev server (skips spawning).
    #[arg(long, env = "ROVO_DEV_BASE_URL")]
    pub base_url: Option<String>,

    /// Host to bind/connect when spawning a server.
    #[arg(long, env = "ROVO_DEV_HOST", default_value = "127.0.0.1")]
    pub host: String,

    /// Port to bind/connect when spawning a server.
    #[arg(long, env = "ROVO_DEV_PORT", default_value_t = 8123)]
    pub port: u16,

    /// Do not spawn the Rovo Dev server; only connect.
    #[arg(long, env = "ROVO_DEV_NO_SPAWN")]
    pub no_spawn: bool,

    /// Rovo Dev CLI command to spawn.
    #[arg(long, env = "ROVO_DEV_COMMAND", default_value = "acli")]
    pub rovo_command: String,

    /// Arguments for the Rovo Dev CLI command (excluding port).
    #[arg(long, env = "ROVO_DEV_ARGS", value_delimiter = ' ')]
    pub rovo_args: Vec<String>,

    /// Enable shadow mode when spawning the server.
    #[arg(long, env = "ROVO_DEV_SHADOW")]
    pub shadow: bool,

    /// Enable deep planning for each prompt.
    #[arg(long, env = "ROVO_DEV_ENABLE_DEEP_PLAN")]
    pub enable_deep_plan: bool,

    /// Seconds to wait for the server healthcheck.
    #[arg(long, env = "ROVO_DEV_STARTUP_TIMEOUT", default_value_t = 30)]
    pub startup_timeout_secs: u64,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub base_url: String,
    pub spawn: bool,
    pub rovo_command: String,
    pub rovo_args: Vec<String>,
    pub port: u16,
    pub shadow: bool,
    pub enable_deep_plan: bool,
    pub startup_timeout: Duration,
}

impl Config {
    pub fn from_cli(cli: Cli) -> Self {
        let spawn = !cli.no_spawn && cli.base_url.is_none();
        let base_url = cli
            .base_url
            .unwrap_or_else(|| format!("http://{}:{}", cli.host, cli.port));
        let rovo_args = if cli.rovo_args.is_empty() {
            default_rovo_args(&cli.rovo_command)
        } else {
            cli.rovo_args
        };

        Self {
            base_url,
            spawn,
            rovo_command: cli.rovo_command,
            rovo_args,
            port: cli.port,
            shadow: cli.shadow,
            enable_deep_plan: cli.enable_deep_plan,
            startup_timeout: Duration::from_secs(cli.startup_timeout_secs),
        }
    }

    pub fn spawn_args(&self) -> Vec<String> {
        let mut args = self.rovo_args.clone();
        args.push(self.port.to_string());
        if self.shadow {
            args.push("--shadow".to_string());
        }
        args
    }
}

fn default_rovo_args(command: &str) -> Vec<String> {
    if is_rovo_binary(command) {
        vec!["serve".to_string()]
    } else {
        vec!["rovodev".to_string(), "serve".to_string()]
    }
}

fn is_rovo_binary(command: &str) -> bool {
    let trimmed = command.trim_end_matches(".exe");
    trimmed.ends_with("rovodev")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_acli_defaults() {
        let cli = Cli {
            base_url: None,
            host: "127.0.0.1".to_string(),
            port: 8123,
            no_spawn: false,
            rovo_command: "acli".to_string(),
            rovo_args: Vec::new(),
            shadow: false,
            enable_deep_plan: false,
            startup_timeout_secs: 30,
        };

        let config = Config::from_cli(cli);
        assert_eq!(config.rovo_args, vec!["rovodev", "serve"]);
    }

    #[test]
    fn uses_rovodev_defaults() {
        let cli = Cli {
            base_url: None,
            host: "127.0.0.1".to_string(),
            port: 8123,
            no_spawn: false,
            rovo_command: "rovodev".to_string(),
            rovo_args: Vec::new(),
            shadow: false,
            enable_deep_plan: false,
            startup_timeout_secs: 30,
        };

        let config = Config::from_cli(cli);
        assert_eq!(config.rovo_args, vec!["serve"]);
    }
}
