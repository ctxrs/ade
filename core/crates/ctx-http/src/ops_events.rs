use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use ctx_avf_linux_runtime::SubstrateLifecycleRecord;
use ctx_fs::permissions::{ensure_private_dir, open_private_append};

use crate::logs;

const OPS_LOG_PREFIX: &str = "ops-events-";
const OPS_LOG_SUFFIX: &str = ".jsonl";

const DEFAULT_RETENTION_DAYS: u64 = 14;
const DEFAULT_MAX_BYTES: u64 = 25 * 1024 * 1024;

const CHANNEL_CAPACITY: usize = 512;

#[derive(Debug, Clone)]
struct OpsEventsConfig {
    enabled: bool,
    local_retention_days: u64,
    local_max_bytes: u64,
}

impl OpsEventsConfig {
    fn from_env() -> Self {
        let enabled = env_bool("CTX_OPS_EVENTS_ENABLED").unwrap_or(true);
        let local_retention_days =
            env_u64("CTX_OPS_EVENTS_LOCAL_RETENTION_DAYS").unwrap_or(DEFAULT_RETENTION_DAYS);
        let local_max_bytes =
            env_u64("CTX_OPS_EVENTS_LOCAL_MAX_BYTES").unwrap_or(DEFAULT_MAX_BYTES);
        Self {
            enabled,
            local_retention_days,
            local_max_bytes,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OpsEvent {
    pub ts: DateTime<Utc>,
    pub level: String,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

impl OpsEvent {
    pub fn new(level: &str, event: &str) -> Self {
        Self {
            ts: Utc::now(),
            level: level.to_string(),
            event: event.to_string(),
            session_id: None,
            worktree_id: None,
            run_id: None,
            turn_id: None,
            provider_id: None,
            tool_kind: None,
            cwd: None,
            worktree_root: None,
            worker_id: None,
            pid: None,
            meta: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SubstrateLifecycleOpsEventContext {
    pub source: &'static str,
    pub workspace_id: Option<String>,
}

pub(crate) fn substrate_lifecycle_observed_event(
    record: &SubstrateLifecycleRecord,
    context: SubstrateLifecycleOpsEventContext,
) -> OpsEvent {
    let mut event = OpsEvent::new("info", "substrate_lifecycle_observed");
    event.meta = Some(serde_json::json!({
        "source": context.source,
        "workspace_id": context.workspace_id,
        "substrate_kind": record.substrate,
        "startup_selection": record.startup_selection,
        "startup_outcome": record.startup_outcome,
        "startup_reason": record.startup_reason,
        "shutdown_outcome": record.shutdown_outcome,
        "shutdown_reason": record.shutdown_reason,
        "restore_attempted": record.restore_attempted,
        "restore_error_present": record.restore_error_present,
        "save_error_present": record.save_error_present,
        "saved_state_written_on_shutdown": record.saved_state_written_on_shutdown,
        "simulated": record.simulated,
    }));
    event
}

#[derive(Clone)]
pub struct OpsEvents {
    tx: mpsc::Sender<OpsEvent>,
}

impl OpsEvents {
    pub fn new(data_root: PathBuf) -> Self {
        let cfg = OpsEventsConfig::from_env();
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        if cfg.enabled {
            tokio::spawn(async move {
                ops_events_worker(data_root, cfg, rx).await;
            });
        }
        Self { tx }
    }

    pub fn emit(&self, event: OpsEvent) {
        let _ = self.tx.try_send(event);
    }
}

async fn ops_events_worker(
    data_root: PathBuf,
    cfg: OpsEventsConfig,
    mut rx: mpsc::Receiver<OpsEvent>,
) {
    let mut last_cleanup: Option<String> = None;
    let mut ticker = tokio::time::interval(Duration::from_secs(3600));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            maybe_event = rx.recv() => {
                let Some(event) = maybe_event else { break; };
                if let Err(err) = append_local_log(&data_root, &event, &cfg).await {
                    tracing::warn!("ops events append failed: {err:#}");
                }
                if cfg.local_retention_days > 0 {
                    let today = event.ts.format("%Y-%m-%d").to_string();
                    if last_cleanup.as_deref() != Some(&today) {
                        let _ = cleanup_old_logs(&data_root, cfg.local_retention_days).await;
                        last_cleanup = Some(today);
                    }
                }
            }
            _ = ticker.tick(), if cfg.local_retention_days > 0 => {
                let today = Utc::now().format("%Y-%m-%d").to_string();
                if last_cleanup.as_deref() != Some(&today) {
                    let _ = cleanup_old_logs(&data_root, cfg.local_retention_days).await;
                    last_cleanup = Some(today);
                }
            }
        }
    }
}

async fn append_local_log(data_root: &Path, event: &OpsEvent, cfg: &OpsEventsConfig) -> Result<()> {
    let dir = logs::logs_dir(data_root);
    ensure_private_dir(&dir).await.ok();
    let date = event.ts.format("%Y-%m-%d").to_string();
    let path = dir.join(format!("{OPS_LOG_PREFIX}{date}{OPS_LOG_SUFFIX}"));

    if cfg.local_max_bytes > 0 {
        if let Ok(metadata) = tokio::fs::metadata(&path).await {
            if metadata.len() >= cfg.local_max_bytes {
                return Ok(());
            }
        }
    }

    let line = serde_json::to_string(event)?;
    let redacted = logs::redact_sensitive(&line);
    let mut file = open_private_append(&path).await?;
    use tokio::io::AsyncWriteExt;
    file.write_all(redacted.as_bytes()).await?;
    file.write_all(b"\n").await?;
    file.flush().await?;
    Ok(())
}

async fn cleanup_old_logs(data_root: &Path, retention_days: u64) -> Result<()> {
    let dir = logs::logs_dir(data_root);
    let mut entries = tokio::fs::read_dir(&dir).await?;
    let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);
    while let Some(entry) = entries.next_entry().await? {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.starts_with(OPS_LOG_PREFIX) || !file_name.ends_with(OPS_LOG_SUFFIX) {
            continue;
        }
        let date = file_name
            .trim_start_matches(OPS_LOG_PREFIX)
            .trim_end_matches(OPS_LOG_SUFFIX);
        let Ok(date) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
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
