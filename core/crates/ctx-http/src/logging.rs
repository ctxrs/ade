use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "daemon-heap-prof")]
use anyhow::anyhow;
use anyhow::Result;
use chrono::{DateTime, Utc};
use tokio::time::MissedTickBehavior;
use tracing::Metadata;
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::Layer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::cli::Commands;

const DEFAULT_DAEMON_LOG_RETENTION_DAYS: u64 = 14;
const DEFAULT_DAEMON_LOG_MAX_BYTES: u64 = 50 * 1024 * 1024;
const DEFAULT_DAEMON_LOG_CHECK_INTERVAL_SECS: u64 = 300;
#[cfg(feature = "daemon-heap-prof")]
const DEFAULT_DAEMON_HEAP_PROFILE_INTERVAL_SECS: u64 = 60;
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
    std::env::var(key)
        .ok()
        .as_deref()
        .and_then(ctx_core::boolish::parse_boolish)
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse().ok())
}

fn env_string(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn daemon_log_path_for_date(logs_dir: &Path, date: &str) -> std::path::PathBuf {
    logs_dir.join(format!("{DAEMON_LOG_PREFIX}{date}"))
}

async fn cleanup_daemon_logs(logs_dir: &Path, retention_days: u64) -> Result<()> {
    let _ = tokio::task::spawn_blocking({
        let logs_dir = logs_dir.to_path_buf();
        move || {
            ctx_fs::permissions::harden_private_directory_files_sync(&logs_dir, |name| {
                name == "daemon.log" || name.starts_with(DAEMON_LOG_PREFIX)
            })
        }
    })
    .await;
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

#[cfg(feature = "daemon-heap-prof")]
fn spawn_daemon_heap_profiler(logs_dir: &Path) {
    if !env_bool("CTX_DAEMON_HEAP_PROFILE").unwrap_or(false) {
        return;
    }
    let interval_secs = env_u64("CTX_DAEMON_HEAP_PROFILE_INTERVAL_SECS")
        .unwrap_or(DEFAULT_DAEMON_HEAP_PROFILE_INTERVAL_SECS)
        .max(1);
    let profile_dir = env_string("CTX_DAEMON_HEAP_PROFILE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| logs_dir.join("daemon-heap"));
    if let Err(err) = std::fs::create_dir_all(&profile_dir) {
        tracing::warn!("heap profile dir create failed: {err:?}");
        return;
    }
    match tikv_jemalloc_ctl::profiling::prof::read() {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!("jemalloc profiling disabled; set MALLOC_CONF=prof:true before start");
            return;
        }
        Err(err) => {
            tracing::warn!("jemalloc profiling unavailable: {err:?}");
            return;
        }
    }

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
            let path = profile_dir.join(format!("heap-{timestamp}.heap"));
            if let Err(err) = dump_heap_profile(&path) {
                tracing::warn!("heap profile dump failed: {err:#}");
            }
        }
    });
}

#[cfg(feature = "daemon-heap-prof")]
fn dump_heap_profile(path: &Path) -> Result<()> {
    use std::ffi::CString;

    let path_str = path.to_string_lossy();
    let c_path = CString::new(path_str.as_bytes())
        .map_err(|err| anyhow!("heap profile path invalid: {err}"))?;
    unsafe {
        tikv_jemalloc_ctl::raw::write(b"prof.dump\0", c_path.as_ptr())
            .map_err(|err| anyhow!("jemalloc prof.dump failed: {err}"))?;
    }
    Ok(())
}

#[cfg(not(feature = "daemon-heap-prof"))]
fn spawn_daemon_heap_profiler(_logs_dir: &Path) {}

pub(crate) fn init_logging_for_command(
    command: &Commands,
) -> Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
    let logs_dir: Option<std::path::PathBuf> = match command {
        Commands::Serve { data_dir, .. } => {
            let data_root = if let Some(p) = data_dir {
                std::path::PathBuf::from(p)
            } else {
                ctx_fs::paths::default_ctx_home()?
            };
            Some(data_root.join("logs"))
        }
        Commands::Init { .. } | Commands::SelfUpdate { .. } => None,
    };

    let Some(logs_dir) = logs_dir else {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
        return Ok(None);
    };

    ctx_fs::permissions::ensure_private_dir_sync(&logs_dir).ok();
    let daemon_log_config = DaemonLogConfig::from_env();
    let file_blocked = Arc::new(AtomicBool::new(false));
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let _ =
        ctx_fs::permissions::open_private_append_sync(&daemon_log_path_for_date(&logs_dir, &today));
    let appender = tracing_appender::rolling::daily(&logs_dir, "daemon.log");
    let (file_writer, file_guard) = tracing_appender::non_blocking(appender);
    let file_writer = ConditionalMakeWriter {
        inner: file_writer,
        blocked: Arc::clone(&file_blocked),
    };

    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(file_writer);

    let stdout_layer = if daemon_log_config.stdout_enabled {
        let stdout_filter = env_string("CTX_DAEMON_LOG_STDOUT_FILTER")
            .and_then(|value| tracing_subscriber::EnvFilter::try_new(value).ok())
            .unwrap_or_else(|| tracing_subscriber::EnvFilter::new("error"));
        Some(
            tracing_subscriber::fmt::layer()
                .with_ansi(true)
                .with_filter(stdout_filter),
        )
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(file_layer)
        .with(stdout_layer)
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    spawn_daemon_log_maintenance(logs_dir.clone(), daemon_log_config, file_blocked);
    spawn_daemon_heap_profiler(&logs_dir);
    Ok(Some(file_guard))
}
