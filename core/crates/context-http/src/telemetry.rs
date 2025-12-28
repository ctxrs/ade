use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::logs;

const TELEMETRY_STATE_FILE: &str = "telemetry.json";
const TELEMETRY_LOG_FILE: &str = "telemetry.jsonl";
const DEFAULT_TELEMETRY_BASE_URL: &str = "https://api.context.rs/functions/v1";

#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub enabled: bool,
    pub endpoint: String,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: default_telemetry_endpoint(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryEventKind {
    WorkspaceRegistered,
    WorkspaceOpened,
    SessionStarted,
    SessionCompleted,
    ProviderCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub name: TelemetryEventKind,
    pub occurred_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
}

impl TelemetryEvent {
    pub fn workspace_registered() -> Self {
        Self {
            name: TelemetryEventKind::WorkspaceRegistered,
            occurred_at: Utc::now(),
            provider_id: None,
            model_id: None,
            env_target: None,
            duration_ms: None,
            status: None,
            success: None,
        }
    }

    pub fn workspace_opened() -> Self {
        Self {
            name: TelemetryEventKind::WorkspaceOpened,
            occurred_at: Utc::now(),
            provider_id: None,
            model_id: None,
            env_target: None,
            duration_ms: None,
            status: None,
            success: None,
        }
    }

    pub fn session_started(
        provider_id: String,
        model_id: String,
        env_target: Option<String>,
    ) -> Self {
        Self {
            name: TelemetryEventKind::SessionStarted,
            occurred_at: Utc::now(),
            provider_id: Some(provider_id),
            model_id: Some(model_id),
            env_target,
            duration_ms: None,
            status: None,
            success: None,
        }
    }

    pub fn session_completed(
        provider_id: String,
        model_id: String,
        env_target: Option<String>,
        status: String,
        duration_ms: u64,
    ) -> Self {
        Self {
            name: TelemetryEventKind::SessionCompleted,
            occurred_at: Utc::now(),
            provider_id: Some(provider_id),
            model_id: Some(model_id),
            env_target,
            duration_ms: Some(duration_ms),
            status: Some(status),
            success: None,
        }
    }

    pub fn provider_call(
        provider_id: String,
        model_id: String,
        env_target: Option<String>,
        success: bool,
        duration_ms: u64,
    ) -> Self {
        Self {
            name: TelemetryEventKind::ProviderCall,
            occurred_at: Utc::now(),
            provider_id: Some(provider_id),
            model_id: Some(model_id),
            env_target,
            duration_ms: Some(duration_ms),
            status: None,
            success: Some(success),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Telemetry {
    tx: mpsc::Sender<TelemetryCommand>,
}

impl Telemetry {
    pub fn new(data_root: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel(512);
        tokio::spawn(async move {
            telemetry_worker(data_root, rx).await;
        });
        Self { tx }
    }

    pub async fn emit(&self, event: TelemetryEvent) {
        let _ = self.tx.send(TelemetryCommand::Event(event)).await;
    }

    pub async fn update_config(&self, cfg: TelemetryConfig) {
        let _ = self.tx.send(TelemetryCommand::UpdateConfig(cfg)).await;
    }

    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(TelemetryCommand::Flush(tx)).await;
        let _ = rx.await;
    }
}

#[derive(Debug)]
enum TelemetryCommand {
    Event(TelemetryEvent),
    UpdateConfig(TelemetryConfig),
    Flush(oneshot::Sender<()>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TelemetryStateFile {
    install_id: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct TelemetryBatch<'a> {
    install_id: &'a str,
    app_version: &'a str,
    os: &'a str,
    arch: &'a str,
    events: &'a [TelemetryEvent],
}

#[derive(Debug, Serialize)]
struct TelemetryLogLine<'a> {
    app_version: &'a str,
    os: &'a str,
    arch: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    install_id: Option<&'a str>,
    event: &'a TelemetryEvent,
}

struct TelemetryRuntime {
    cfg: TelemetryConfig,
    install_id: Option<String>,
    buffer: Vec<TelemetryEvent>,
    app_version: String,
    os: String,
    arch: String,
    client: reqwest::Client,
}

pub fn default_telemetry_endpoint() -> String {
    let base = std::env::var("CONTEXT_TELEMETRY_BASE_URL")
        .unwrap_or_else(|_| DEFAULT_TELEMETRY_BASE_URL.to_string());
    format!("{}/telemetry", base.trim_end_matches('/'))
}

fn telemetry_state_path(data_root: &Path) -> PathBuf {
    data_root.join(TELEMETRY_STATE_FILE)
}

fn telemetry_log_path(data_root: &Path) -> PathBuf {
    logs::logs_dir(data_root).join(TELEMETRY_LOG_FILE)
}

async fn load_or_create_install_id(data_root: &Path) -> Option<String> {
    let path = telemetry_state_path(data_root);
    if let Ok(raw) = tokio::fs::read_to_string(&path).await {
        if let Ok(state) = serde_json::from_str::<TelemetryStateFile>(&raw) {
            if !state.install_id.trim().is_empty() {
                return Some(state.install_id);
            }
        }
    }

    let install_id = uuid::Uuid::new_v4().to_string();
    let state = TelemetryStateFile {
        install_id: install_id.clone(),
        created_at: Utc::now(),
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(&state) {
        let _ = tokio::fs::write(&path, bytes).await;
    }
    Some(install_id)
}

async fn append_local_log_with_root(
    data_root: &Path,
    runtime: &TelemetryRuntime,
    event: &TelemetryEvent,
) -> Result<()> {
    let path = telemetry_log_path(data_root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }

    let line = TelemetryLogLine {
        app_version: &runtime.app_version,
        os: &runtime.os,
        arch: &runtime.arch,
        install_id: runtime.install_id.as_deref(),
        event,
    };
    let payload = serde_json::to_string(&line)?;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await?;
    use tokio::io::AsyncWriteExt;
    file.write_all(payload.as_bytes()).await?;
    file.write_all(b"\n").await?;
    file.flush().await?;
    Ok(())
}

async fn send_batch(runtime: &TelemetryRuntime, events: &[TelemetryEvent]) -> Result<()> {
    let install_id = match runtime.install_id.as_deref() {
        Some(id) if runtime.cfg.enabled => id,
        _ => return Ok(()),
    };
    let batch = TelemetryBatch {
        install_id,
        app_version: &runtime.app_version,
        os: &runtime.os,
        arch: &runtime.arch,
        events,
    };
    runtime
        .client
        .post(runtime.cfg.endpoint.as_str())
        .json(&batch)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn telemetry_worker(data_root: PathBuf, mut rx: mpsc::Receiver<TelemetryCommand>) {
    let mut runtime = TelemetryRuntime {
        cfg: TelemetryConfig::default(),
        install_id: None,
        buffer: Vec::new(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        client: reqwest::Client::new(),
    };

    let mut flush_tick = tokio::time::interval(Duration::from_secs(10));
    const MAX_BUFFER: usize = 1000;
    const FLUSH_BATCH: usize = 32;

    loop {
        tokio::select! {
            _ = flush_tick.tick() => {
                if runtime.cfg.enabled && !runtime.buffer.is_empty() {
                    let batch = runtime.buffer.drain(..runtime.buffer.len().min(FLUSH_BATCH)).collect::<Vec<_>>();
                    if send_batch(&runtime, &batch).await.is_err() {
                        runtime.buffer.splice(0..0, batch);
                        if runtime.buffer.len() > MAX_BUFFER {
                            runtime.buffer.truncate(MAX_BUFFER);
                        }
                    }
                }
            }
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    TelemetryCommand::Event(event) => {
                        let _ = append_local_log_with_root(&data_root, &runtime, &event).await;
                        if runtime.cfg.enabled {
                            if runtime.install_id.is_none() {
                                runtime.install_id = load_or_create_install_id(&data_root).await;
                            }
                            runtime.buffer.push(event);
                            if runtime.buffer.len() >= FLUSH_BATCH {
                                let batch = runtime.buffer.drain(..).collect::<Vec<_>>();
                                if send_batch(&runtime, &batch).await.is_err() {
                                    runtime.buffer = batch;
                                    if runtime.buffer.len() > MAX_BUFFER {
                                        runtime.buffer.truncate(MAX_BUFFER);
                                    }
                                }
                            }
                        }
                    }
                    TelemetryCommand::UpdateConfig(cfg) => {
                        runtime.cfg = cfg;
                        if runtime.cfg.enabled && runtime.install_id.is_none() {
                            runtime.install_id = load_or_create_install_id(&data_root).await;
                        }
                        if !runtime.cfg.enabled {
                            runtime.buffer.clear();
                        }
                    }
                    TelemetryCommand::Flush(done) => {
                        if runtime.cfg.enabled && !runtime.buffer.is_empty() {
                            let batch = runtime.buffer.drain(..).collect::<Vec<_>>();
                            let _ = send_batch(&runtime, &batch).await;
                        }
                        let _ = done.send(());
                    }
                }
            }
        }
    }
}
