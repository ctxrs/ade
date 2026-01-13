use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use chrono::{DateTime, Utc};
use tracing::{Metadata, warn};
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tokio::time::MissedTickBehavior;

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

const DEFAULT_DAEMON_LOG_RETENTION_DAYS: u64 = 14;
const DEFAULT_DAEMON_LOG_MAX_BYTES: u64 = 50 * 1024 * 1024;
const DEFAULT_DAEMON_LOG_CHECK_INTERVAL_SECS: u64 = 300;
const DAEMON_LOG_PREFIX: &str = "daemon.log.";

struct ConditionalWriter<W> {
    inner: W,
    blocked: Arc<AtomicBool>,
}

impl<W: io::Write> io::Write for ConditionalWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.blocked.load(Ordering::Relaxed) {
            Ok(buf.len())
        } else {
            self.inner.write(buf)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.blocked.load(Ordering::Relaxed) {
            Ok(())
        } else {
            self.inner.flush()
        }
    }
}

struct ConditionalMakeWriter<W> {
    inner: W,
    blocked: Arc<AtomicBool>,
}

impl<'a, W> MakeWriter<'a> for ConditionalMakeWriter<W>
where
    W: MakeWriter<'a>,
{
    type Writer = ConditionalWriter<W::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        ConditionalWriter {
            inner: self.inner.make_writer(),
            blocked: Arc::clone(&self.blocked),
        }
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Self::Writer {
        ConditionalWriter {
            inner: self.inner.make_writer_for(meta),
            blocked: Arc::clone(&self.blocked),
        }
    }
}

#[derive(Debug, Clone)]
struct DaemonLogConfig {
    retention_days: u64,
    max_bytes: u64,
    stdout_enabled: bool,
    check_interval: Duration,
}

impl DaemonLogConfig {
    fn from_env() -> Self {
        let retention_days =
            env_u64("CTX_DAEMON_LOG_RETENTION_DAYS").unwrap_or(DEFAULT_DAEMON_LOG_RETENTION_DAYS);
        let max_bytes = env_u64("CTX_DAEMON_LOG_MAX_BYTES").unwrap_or(DEFAULT_DAEMON_LOG_MAX_BYTES);
        let stdout_enabled = env_bool("CTX_DAEMON_LOG_STDOUT").unwrap_or(false);
        Self {
            retention_days,
            max_bytes,
            stdout_enabled,
            check_interval: Duration::from_secs(DEFAULT_DAEMON_LOG_CHECK_INTERVAL_SECS),
        }
    }
}

fn env_bool(key: &str) -> Option<bool> {
    std::env::var(key).ok().map(|raw| {
        let trimmed = raw.trim();
        trimmed == "1" || trimmed.eq_ignore_ascii_case("true") || trimmed.eq_ignore_ascii_case("yes")
    })
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key).ok().and_then(|raw| raw.trim().parse().ok())
}

fn daemon_log_path_for_date(logs_dir: &Path, date: &str) -> std::path::PathBuf {
    logs_dir.join(format!("{DAEMON_LOG_PREFIX}{date}"))
}

async fn cleanup_daemon_logs(logs_dir: &Path, retention_days: u64) -> Result<()> {
    let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);
    let mut entries = tokio::fs::read_dir(logs_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name == "daemon.log" {
            if let Ok(metadata) = entry.metadata().await {
                if let Ok(modified) = metadata.modified() {
                    let modified = DateTime::<Utc>::from(modified);
                    if modified < cutoff {
                        let _ = tokio::fs::remove_file(entry.path()).await;
                    }
                }
            }
            continue;
        }
        if !file_name.starts_with(DAEMON_LOG_PREFIX) {
            continue;
        }
        let date_str = file_name.trim_start_matches(DAEMON_LOG_PREFIX);
        let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") else {
            continue;
        };
        let naive = date.and_hms_opt(0, 0, 0).unwrap_or_default();
        let date = DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc);
        if date < cutoff {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
    Ok(())
}

fn spawn_daemon_log_maintenance(
    logs_dir: std::path::PathBuf,
    cfg: DaemonLogConfig,
    file_blocked: Arc<AtomicBool>,
) {
    if cfg.retention_days == 0 && cfg.max_bytes == 0 {
        return;
    }
    tokio::spawn(async move {
        let mut last_cleanup = None::<String>;
        let mut ticker = tokio::time::interval(cfg.check_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            let today = Utc::now().format("%Y-%m-%d").to_string();
            if cfg.retention_days > 0 && last_cleanup.as_deref() != Some(&today) {
                if let Err(err) = cleanup_daemon_logs(&logs_dir, cfg.retention_days).await {
                    tracing::warn!("daemon log cleanup failed: {err:#}");
                }
                last_cleanup = Some(today.clone());
            }
            if cfg.max_bytes > 0 {
                let path = daemon_log_path_for_date(&logs_dir, &today);
                let over = tokio::fs::metadata(&path)
                    .await
                    .map(|meta| meta.len() >= cfg.max_bytes)
                    .unwrap_or(false);
                file_blocked.store(over, Ordering::Relaxed);
            } else {
                file_blocked.store(false, Ordering::Relaxed);
            }
            ticker.tick().await;
        }
    });
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
        let daemon_log_config = DaemonLogConfig::from_env();
        let file_blocked = Arc::new(AtomicBool::new(false));
        let appender = tracing_appender::rolling::daily(logs_dir, "daemon.log");
        let (file_writer, file_guard) = tracing_appender::non_blocking(appender);
        _file_guard = Some(file_guard);
        let file_writer = ConditionalMakeWriter {
            inner: file_writer,
            blocked: Arc::clone(&file_blocked),
        };

        let file_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(file_writer);

        if daemon_log_config.stdout_enabled {
            tracing_subscriber::registry()
                .with(file_layer)
                .with(tracing_subscriber::fmt::layer().with_ansi(true))
                .with(tracing_subscriber::EnvFilter::from_default_env())
                .init();
        } else {
            tracing_subscriber::registry()
                .with(file_layer)
                .with(tracing_subscriber::EnvFilter::from_default_env())
                .init();
        }

        spawn_daemon_log_maintenance(logs_dir.clone(), daemon_log_config, file_blocked);
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    }

    if let Err(err) = rustls::crypto::aws_lc_rs::default_provider().install_default() {
        warn!("failed to install rustls crypto provider: {err:?}");
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
