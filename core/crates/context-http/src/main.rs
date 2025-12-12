use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "context")]
#[command(about = "Context daemon and CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Serve {
        #[arg(long, default_value = "127.0.0.1:4399")]
        bind: String,
        #[arg(long)]
        data_dir: Option<String>,
    },
    Init {
        #[arg(long)]
        root: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Serve { bind, data_dir } => {
            context_http::daemon::serve(bind, data_dir).await?;
        }
        Commands::Init { root } => {
            context_http::daemon::init_workspace(root).await?;
        }
    }
    Ok(())
}

