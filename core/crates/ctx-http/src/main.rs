use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "ctx")]
#[command(about = "ctx daemon and CLI", long_about = None)]
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
    SelfUpdate {
        /// Release channel (e.g. stable, nightly)
        #[arg(long, default_value = "stable")]
        channel: String,
        /// Base URL for release manifests and downloads (e.g. https://api.ctx.rs/functions/v1)
        #[arg(long)]
        base_url: Option<String>,
        /// Run non-interactively.
        #[arg(long)]
        yes: bool,
        /// Only check whether an update exists; do not download/apply.
        #[arg(long)]
        check: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut _file_guard: Option<tracing_appender::non_blocking::WorkerGuard> = None;

    // Initialize logging early so daemon startup failures land in a predictable file.
    let logs_dir: Option<std::path::PathBuf> = match &cli.command {
        Commands::Serve { data_dir, .. } => {
            let data_root = if let Some(p) = data_dir {
                std::path::PathBuf::from(p)
            } else {
                let base =
                    directories::BaseDirs::new().ok_or_else(|| anyhow!("resolving home dir"))?;
                base.home_dir().join(".ctx")
            };
            Some(data_root.join("logs"))
        }
        Commands::Init { .. } => None,
        Commands::SelfUpdate { .. } => None,
    };

    if let Some(logs_dir) = &logs_dir {
        std::fs::create_dir_all(logs_dir).ok();
        let appender = tracing_appender::rolling::never(logs_dir, "daemon.log");
        let (file_writer, file_guard) = tracing_appender::non_blocking(appender);
        _file_guard = Some(file_guard);

        let env_filter = tracing_subscriber::EnvFilter::from_default_env();
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().with_ansi(true))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(file_writer),
            )
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    }

    match cli.command {
        Commands::Serve { bind, data_dir } => {
            ctx_http::daemon::serve(bind, data_dir).await?;
        }
        Commands::Init { root } => {
            ctx_http::daemon::init_workspace(root).await?;
        }
        Commands::SelfUpdate {
            channel,
            base_url,
            yes,
            check,
        } => {
            let base_url = base_url.unwrap_or_else(ctx_http::updates::default_download_base_url);
            ctx_http::updates::self_update_daemon(&channel, &base_url, yes, check).await?;
        }
    }
    Ok(())
}
