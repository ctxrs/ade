use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct LogFileInfo {
    pub name: String,
    pub bytes: u64,
    pub modified_utc: Option<String>,
}

pub fn logs_dir(data_root: &Path) -> PathBuf {
    data_root.join("logs")
}

pub fn daemon_log_path(data_root: &Path) -> PathBuf {
    logs_dir(data_root).join("daemon.log")
}

pub fn desktop_log_path(data_root: &Path) -> PathBuf {
    logs_dir(data_root).join("desktop.log")
}

pub fn redact_sensitive(input: &str) -> String {
    fn redact_after_marker(mut s: String, marker: &str) -> String {
        let redacted = "[REDACTED]";
        let mut search_from = 0usize;
        while let Some(rel) = s[search_from..].find(marker) {
            let marker_start = search_from + rel;
            let start = marker_start + marker.len();
            if start >= s.len() {
                break;
            }
            if s[start..].starts_with(redacted) {
                search_from = start + redacted.len();
                continue;
            }

            let mut end = s.len();
            for (i, ch) in s[start..].char_indices() {
                if ch.is_whitespace() || ch == '"' || ch == '\'' || ch == '&' {
                    end = start + i;
                    break;
                }
            }

            s.replace_range(start..end, redacted);
            search_from = start + redacted.len();
        }
        s
    }

    let mut out = input.to_string();
    out = redact_after_marker(out, "Bearer ");
    out = redact_after_marker(out, "bearer ");
    out = redact_after_marker(out, "Authorization: Bearer ");
    out = redact_after_marker(out, "authorization: Bearer ");
    out = redact_after_marker(out, "token=");
    out = redact_after_marker(out, "TOKEN=");
    out = redact_after_marker(out, "CONTEXT_DESKTOP_TOKEN=");
    out = redact_after_marker(out, "contextAuthToken\":\"");
    out = redact_after_marker(out, "context_auth_token\":\"");
    out
}

pub async fn append_desktop_log_line(data_root: &Path, line: &str) -> Result<()> {
    let log_dir = logs_dir(data_root);
    tokio::fs::create_dir_all(&log_dir).await.ok();

    let redacted = redact_sensitive(line);
    let path = desktop_log_path(data_root);
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
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
        let Ok(ft) = ent.file_type().await else { continue };
        if !ft.is_file() {
            continue;
        }
        let Ok(md) = ent.metadata().await else { continue };

        let modified_utc = md.modified().ok().map(|t| {
            DateTime::<Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        });

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
    tokio::fs::create_dir_all(&dir).await.ok();

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

