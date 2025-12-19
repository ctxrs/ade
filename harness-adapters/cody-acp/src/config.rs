use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "cody-acp", version, about = "ACP stdio adapter for the Cody CLI")]
pub struct Cli {
    /// Path to the Cody CLI binary.
    #[arg(long, env = "CODY_COMMAND", default_value = "cody")]
    pub cody_command: String,

    /// Arguments to pass to the Cody CLI.
    #[arg(
        long,
        env = "CODY_ARGS",
        value_delimiter = ' ',
        default_value = "api jsonrpc-stdio"
    )]
    pub cody_args: Vec<String>,

    /// Override the Sourcegraph endpoint for Cody.
    #[arg(long, env = "SRC_ENDPOINT")]
    pub endpoint: Option<String>,

    /// Override the Sourcegraph access token for Cody.
    #[arg(long, env = "SRC_ACCESS_TOKEN")]
    pub access_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub cody_command: String,
    pub cody_args: Vec<String>,
    pub endpoint: Option<String>,
    pub access_token: Option<String>,
}

impl Config {
    pub fn from_cli(cli: Cli) -> Self {
        Self {
            cody_command: cli.cody_command,
            cody_args: cli.cody_args,
            endpoint: cli.endpoint,
            access_token: cli.access_token,
        }
    }

    pub fn resolved_endpoint(&self) -> String {
        if let Some(endpoint) = self.endpoint.clone() {
            return endpoint;
        }
        std::env::var("SRC_ENDPOINT").unwrap_or_else(|_| "https://sourcegraph.com".to_string())
    }

    pub fn resolved_access_token(&self) -> Option<String> {
        if let Some(token) = self.access_token.clone() {
            return Some(token);
        }
        std::env::var("SRC_ACCESS_TOKEN").ok()
    }
}
