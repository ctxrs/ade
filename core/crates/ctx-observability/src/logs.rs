use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use ctx_fs::permissions::{
    ensure_private_dir, ensure_private_dir_sync, harden_private_directory_files_sync,
    open_private_append, open_private_append_sync,
};
use serde::Serialize;
use tokio::time::MissedTickBehavior;

const DEFAULT_DAEMON_LOG_RETENTION_DAYS: u64 = 14;
const DEFAULT_DAEMON_LOG_MAX_BYTES: u64 = 50 * 1024 * 1024;
const DEFAULT_DAEMON_LOG_CHECK_INTERVAL_SECS: u64 = 300;
const DAEMON_LOG_PREFIX: &str = "daemon.log.";

#[derive(Debug, Clone)]
pub struct DaemonLogConfig {
    pub retention_days: u64,
    pub max_bytes: u64,
    pub stdout_enabled: bool,
    check_interval: Duration,
}

impl DaemonLogConfig {
    pub fn from_env() -> Self {
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

#[derive(Debug, Clone, Serialize)]
pub struct LogFileInfo {
    pub name: String,
    pub bytes: u64,
    pub modified_utc: Option<String>,
}

pub fn logs_dir(data_root: &Path) -> PathBuf {
    data_root.join("logs")
}

pub fn default_ctx_logs_dir() -> Result<PathBuf> {
    Ok(ctx_fs::paths::default_ctx_home()?.join("logs"))
}

pub fn desktop_log_path(data_root: &Path) -> PathBuf {
    logs_dir(data_root).join("desktop.log")
}

pub fn daemon_log_path_for_date(logs_dir: &Path, date: &str) -> PathBuf {
    logs_dir.join(format!("{DAEMON_LOG_PREFIX}{date}"))
}

pub fn prepare_daemon_log_file_for_today_sync(logs_dir: &Path) {
    ensure_private_dir_sync(logs_dir).ok();
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let _ = open_private_append_sync(&daemon_log_path_for_date(logs_dir, &today));
}

pub fn spawn_daemon_log_maintenance(
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

pub fn redact_sensitive(input: &str) -> String {
    ctx_core::redaction::redact_sensitive(input)
}

pub async fn append_desktop_log_line(data_root: &Path, line: &str) -> Result<()> {
    let log_dir = logs_dir(data_root);
    ensure_private_dir(&log_dir).await.ok();

    let redacted = redact_sensitive(line);
    let path = desktop_log_path(data_root);
    let mut file = open_private_append(&path)
        .await
        .with_context(|| format!("opening desktop log at {}", path.display()))?;

    use tokio::io::AsyncWriteExt;
    file.write_all(redacted.as_bytes()).await?;
    if !redacted.ends_with('\n') {
        file.write_all(b"\n").await?;
    }
    file.flush().await?;
    Ok(())
}

pub async fn list_log_files(data_root: &Path) -> Vec<LogFileInfo> {
    let dir = logs_dir(data_root);
    let mut entries = Vec::new();

    let mut rd = match tokio::fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(_) => return entries,
    };

    while let Ok(Some(ent)) = rd.next_entry().await {
        let Ok(ft) = ent.file_type().await else {
            continue;
        };
        if !ft.is_file() {
            continue;
        }
        let Ok(md) = ent.metadata().await else {
            continue;
        };

        let modified_utc = md
            .modified()
            .ok()
            .map(|t| DateTime::<Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true));

        entries.push(LogFileInfo {
            name: ent.file_name().to_string_lossy().to_string(),
            bytes: md.len(),
            modified_utc,
        });
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

pub async fn open_logs_folder(data_root: &Path) -> Result<()> {
    let dir = logs_dir(data_root);
    ensure_private_dir(&dir).await.ok();

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        anyhow::bail!("open_logs_folder not supported on this platform");
    }

    #[cfg(target_os = "macos")]
    let mut cmd = tokio::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut cmd = tokio::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut cmd = tokio::process::Command::new("explorer");

    cmd.arg(&dir);
    let status = cmd.status().await.context("spawning open command")?;
    if !status.success() {
        anyhow::bail!("failed to open logs folder (exit={status})");
    }
    Ok(())
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

async fn cleanup_daemon_logs(logs_dir: &Path, retention_days: u64) -> Result<()> {
    let _ = tokio::task::spawn_blocking({
        let logs_dir = logs_dir.to_path_buf();
        move || {
            harden_private_directory_files_sync(&logs_dir, |name| {
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

#[cfg(test)]
mod tests {
    use super::{daemon_log_path_for_date, redact_sensitive};

    #[test]
    fn redact_sensitive_covers_scoped_mcp_tokens() {
        let input = concat!(
            "CTX_MCP_TOKEN=env-secret ",
            "{\"CTX_MCP_TOKEN\":\"json-secret\"} ",
            "{\"CTX_MCP_TOKEN\": \"json-spaced-secret\"} ",
            "{\"ctx_mcp_token\":\"lower-secret\"} ",
            "{\"ctx_mcp_token\": \"lower-spaced-secret\"} ",
            "{\"ctxMcpToken\":\"camel-secret\"}"
        );

        let redacted = redact_sensitive(input);

        for secret in [
            "env-secret",
            "json-secret",
            "json-spaced-secret",
            "lower-secret",
            "lower-spaced-secret",
            "camel-secret",
        ] {
            assert!(!redacted.contains(secret), "{secret} leaked: {redacted}");
        }
        assert_eq!(redacted.matches("[REDACTED]").count(), 6);
    }

    #[test]
    fn daemon_log_path_uses_daily_daemon_prefix() {
        let path = daemon_log_path_for_date(std::path::Path::new("/tmp/ctx/logs"), "2026-05-12");

        assert_eq!(
            path,
            std::path::PathBuf::from("/tmp/ctx/logs/daemon.log.2026-05-12")
        );
    }
}
