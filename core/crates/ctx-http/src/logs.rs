use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use ctx_fs::permissions::{ensure_private_dir, open_private_append};
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

pub fn desktop_log_path(data_root: &Path) -> PathBuf {
    logs_dir(data_root).join("desktop.log")
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

#[cfg(test)]
mod tests {
    use super::redact_sensitive;

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
}
