use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use tokio::time::MissedTickBehavior;

use super::{env_bool, env_u64};

const DEFAULT_DAEMON_LOG_RETENTION_DAYS: u64 = 14;
const DEFAULT_DAEMON_LOG_MAX_BYTES: u64 = 50 * 1024 * 1024;
const DEFAULT_DAEMON_LOG_CHECK_INTERVAL_SECS: u64 = 300;
const DAEMON_LOG_PREFIX: &str = "daemon.log.";

#[derive(Debug, Clone)]
pub(super) struct DaemonLogConfig {
    pub(super) retention_days: u64,
    pub(super) max_bytes: u64,
    pub(super) stdout_enabled: bool,
    check_interval: Duration,
}

impl DaemonLogConfig {
    pub(super) fn from_env() -> Self {
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

pub(super) fn daemon_log_path_for_date(logs_dir: &Path, date: &str) -> PathBuf {
    logs_dir.join(format!("{DAEMON_LOG_PREFIX}{date}"))
}

pub(super) fn spawn_daemon_log_maintenance(
    logs_dir: PathBuf,
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
