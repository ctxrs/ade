use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

use crate::logs;

const TELEMETRY_STATE_FILE: &str = "telemetry.json";
const TELEMETRY_LOG_FILE: &str = "telemetry.jsonl";
const DEFAULT_TELEMETRY_BASE_URL: &str = "https://api.ctx.rs/functions/v1";
const TELEMETRY_CHANNEL_SEND_TIMEOUT: Duration = Duration::from_millis(500);
const TELEMETRY_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

pub type TelemetryProperties = Map<String, Value>;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryPlane {
    Product,
    Incident,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryDelivery {
    Remote,
    LocalOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryOriginRuntime {
    Web,
    Desktop,
    MobileShell,
    Daemon,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub event_id: String,
    pub event_name: String,
    pub event_version: u32,
    pub occurred_at: DateTime<Utc>,
    pub plane: TelemetryPlane,
    pub delivery: TelemetryDelivery,
    pub origin_runtime: TelemetryOriginRuntime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_install_id: Option<String>,
    pub app_version: String,
    pub os: String,
    pub arch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "TelemetryProperties::is_empty")]
    pub properties: TelemetryProperties,
}

impl TelemetryEvent {
    pub fn daemon_product(event_name: impl Into<String>) -> Self {
        Self::daemon_event(event_name, TelemetryPlane::Product)
    }

    pub fn daemon_incident(event_name: impl Into<String>) -> Self {
        Self::daemon_event(event_name, TelemetryPlane::Incident)
    }

    fn daemon_event(event_name: impl Into<String>, plane: TelemetryPlane) -> Self {
        Self {
            event_id: uuid::Uuid::new_v4().to_string(),
            event_name: event_name.into(),
            event_version: 1,
            occurred_at: Utc::now(),
            plane,
            delivery: TelemetryDelivery::Remote,
            origin_runtime: TelemetryOriginRuntime::Daemon,
            origin_install_id: None,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            surface: None,
            env_target: None,
            source: None,
            properties: TelemetryProperties::new(),
        }
    }

    pub fn local_only(mut self) -> Self {
        self.delivery = TelemetryDelivery::LocalOnly;
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        let source = source.into();
        if !source.trim().is_empty() {
            self.source = Some(source);
        }
        self
    }

    pub fn with_surface(mut self, surface: impl Into<String>) -> Self {
        let surface = surface.into();
        if !surface.trim().is_empty() {
            self.surface = Some(surface);
        }
        self
    }

    pub fn with_env_target(mut self, env_target: impl Into<String>) -> Self {
        let env_target = env_target.into();
        if !env_target.trim().is_empty() {
            self.env_target = Some(env_target);
        }
        self
    }

    pub fn with_property(mut self, key: impl Into<String>, value: Value) -> Self {
        let key = key.into();
        if key.trim().is_empty() {
            return self;
        }
        self.properties.insert(key, value);
        self
    }

    pub fn with_properties(mut self, properties: TelemetryProperties) -> Self {
        self.properties.extend(properties);
        self
    }

    pub fn workspace_registered() -> Self {
        Self::daemon_product("workspace_registered")
    }

    pub fn workspace_opened() -> Self {
        Self::daemon_product("workspace_opened")
    }

    pub fn session_started(
        provider_id: String,
        model_id: String,
        execution_environment: Option<String>,
        session_root_kind: Option<String>,
    ) -> Self {
        let mut event = Self::daemon_product("session_started")
            .with_property("provider_id", json!(provider_id))
            .with_property("model_id", json!(model_id));
        if let Some(env_target) = execution_environment {
            event = event.with_env_target(env_target.clone());
            event = event.with_property("execution_environment", json!(env_target));
        }
        if let Some(session_root_kind) = session_root_kind {
            event = event.with_property("session_root_kind", json!(session_root_kind));
        }
        event
    }

    pub fn session_completed(
        provider_id: String,
        model_id: String,
        execution_environment: Option<String>,
        session_root_kind: Option<String>,
        status: String,
        duration_ms: u64,
    ) -> Self {
        let mut event = Self::daemon_product("session_completed")
            .with_property("provider_id", json!(provider_id))
            .with_property("model_id", json!(model_id))
            .with_property("status", json!(status))
            .with_property("duration_ms", json!(duration_ms));
        if let Some(env_target) = execution_environment {
            event = event.with_env_target(env_target.clone());
            event = event.with_property("execution_environment", json!(env_target));
        }
        if let Some(session_root_kind) = session_root_kind {
            event = event.with_property("session_root_kind", json!(session_root_kind));
        }
        event
    }

    pub fn session_interrupt_latency(
        provider_id: String,
        model_id: String,
        execution_environment: Option<String>,
        session_root_kind: Option<String>,
        duration_ms: u64,
        duration_bucket: String,
    ) -> Self {
        let mut event = Self::daemon_product("session_interrupt_latency")
            .with_property("provider_id", json!(provider_id))
            .with_property("model_id", json!(model_id))
            .with_property("duration_ms", json!(duration_ms))
            .with_property("duration_bucket", json!(duration_bucket))
            .with_property("status", json!("interrupted"))
            .with_property("success", json!(true));
        if let Some(env_target) = execution_environment {
            event = event.with_env_target(env_target.clone());
            event = event.with_property("execution_environment", json!(env_target));
        }
        if let Some(session_root_kind) = session_root_kind {
            event = event.with_property("session_root_kind", json!(session_root_kind));
        }
        event
    }

    pub fn provider_call(
        provider_id: String,
        model_id: String,
        execution_environment: Option<String>,
        session_root_kind: Option<String>,
        success: bool,
        duration_ms: u64,
    ) -> Self {
        let mut event = Self::daemon_product("provider_call")
            .with_property("provider_id", json!(provider_id))
            .with_property("model_id", json!(model_id))
            .with_property("success", json!(success))
            .with_property("duration_ms", json!(duration_ms));
        if let Some(env_target) = execution_environment {
            event = event.with_env_target(env_target.clone());
            event = event.with_property("execution_environment", json!(env_target));
        }
        if let Some(session_root_kind) = session_root_kind {
            event = event.with_property("session_root_kind", json!(session_root_kind));
        }
        event
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
        match timeout(
            TELEMETRY_CHANNEL_SEND_TIMEOUT,
            self.tx.send(TelemetryCommand::Event(event)),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => tracing::warn!("telemetry channel closed; dropping event"),
            Err(_) => tracing::warn!("telemetry channel blocked; dropping event"),
        }
    }

    pub async fn emit_many(&self, events: Vec<TelemetryEvent>) {
        if events.is_empty() {
            return;
        }
        match timeout(
            TELEMETRY_CHANNEL_SEND_TIMEOUT,
            self.tx.send(TelemetryCommand::Events(events)),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => tracing::warn!("telemetry channel closed; dropping event batch"),
            Err(_) => tracing::warn!("telemetry channel blocked; dropping event batch"),
        }
    }

    pub async fn update_config(&self, cfg: TelemetryConfig) {
        match timeout(
            TELEMETRY_CHANNEL_SEND_TIMEOUT,
            self.tx.send(TelemetryCommand::UpdateConfig(cfg)),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => tracing::warn!("telemetry channel closed; dropping config update"),
            Err(_) => tracing::warn!("telemetry channel blocked; dropping config update"),
        }
    }

    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        match timeout(
            TELEMETRY_CHANNEL_SEND_TIMEOUT,
            self.tx.send(TelemetryCommand::Flush(tx)),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                tracing::warn!("telemetry channel closed; flush skipped");
                return;
            }
            Err(_) => {
                tracing::warn!("telemetry channel blocked; flush skipped");
                return;
            }
        }
        let _ = timeout(TELEMETRY_REQUEST_TIMEOUT, rx).await;
    }
}

#[derive(Debug)]
enum TelemetryCommand {
    Event(TelemetryEvent),
    Events(Vec<TelemetryEvent>),
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
    broker_install_id: &'a str,
    broker_runtime: &'static str,
    broker_app_version: &'a str,
    broker_os: &'a str,
    broker_arch: &'a str,
    events: &'a [TelemetryEvent],
}

#[derive(Debug, Serialize)]
struct TelemetryLogLine<'a> {
    broker_install_id: Option<&'a str>,
    broker_runtime: &'static str,
    broker_app_version: &'a str,
    broker_os: &'a str,
    broker_arch: &'a str,
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
    let base = std::env::var("CTX_TELEMETRY_BASE_URL")
        .unwrap_or_else(|_| DEFAULT_TELEMETRY_BASE_URL.to_string());
    format!("{}/telemetry", base.trim_end_matches('/'))
}

fn telemetry_state_path(data_root: &Path) -> PathBuf {
    data_root.join(TELEMETRY_STATE_FILE)
}

fn telemetry_log_path(data_root: &Path) -> PathBuf {
    logs::logs_dir(data_root).join(TELEMETRY_LOG_FILE)
}

async fn send_batch_with_timeout(runtime: &TelemetryRuntime, batch: &[TelemetryEvent]) -> bool {
    matches!(
        timeout(TELEMETRY_REQUEST_TIMEOUT, send_batch(runtime, batch)).await,
        Ok(Ok(()))
    )
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

async fn ensure_broker_install_id(
    runtime: &mut TelemetryRuntime,
    data_root: &Path,
) -> Option<String> {
    if runtime.install_id.is_none() {
        runtime.install_id = load_or_create_install_id(data_root).await;
    }
    runtime.install_id.clone()
}

fn populate_daemon_origin_install_id(event: &mut TelemetryEvent, broker_install_id: Option<&str>) {
    if event.origin_runtime == TelemetryOriginRuntime::Daemon && event.origin_install_id.is_none() {
        event.origin_install_id = broker_install_id.map(ToString::to_string);
    }
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
        broker_install_id: runtime.install_id.as_deref(),
        broker_runtime: "daemon",
        broker_app_version: &runtime.app_version,
        broker_os: &runtime.os,
        broker_arch: &runtime.arch,
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
    let broker_install_id = match runtime.install_id.as_deref() {
        Some(id) if runtime.cfg.enabled => id,
        _ => return Ok(()),
    };
    let batch = TelemetryBatch {
        broker_install_id,
        broker_runtime: "daemon",
        broker_app_version: &runtime.app_version,
        broker_os: &runtime.os,
        broker_arch: &runtime.arch,
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

async fn flush_remote_buffer(runtime: &mut TelemetryRuntime) {
    if !runtime.cfg.enabled || runtime.buffer.is_empty() {
        return;
    }
    let batch = runtime.buffer.drain(..).collect::<Vec<_>>();
    if !send_batch_with_timeout(runtime, &batch).await {
        runtime.buffer = batch;
        const MAX_BUFFER: usize = 1000;
        if runtime.buffer.len() > MAX_BUFFER {
            runtime.buffer.truncate(MAX_BUFFER);
        }
    }
}

async fn process_event(
    runtime: &mut TelemetryRuntime,
    data_root: &Path,
    mut event: TelemetryEvent,
) {
    let broker_install_id = if event.origin_runtime == TelemetryOriginRuntime::Daemon
        || event.delivery == TelemetryDelivery::Remote
    {
        ensure_broker_install_id(runtime, data_root).await
    } else {
        runtime.install_id.clone()
    };
    populate_daemon_origin_install_id(&mut event, broker_install_id.as_deref());

    let _ = append_local_log_with_root(data_root, runtime, &event).await;

    if event.delivery == TelemetryDelivery::LocalOnly || !runtime.cfg.enabled {
        return;
    }

    runtime.buffer.push(event);
    const FLUSH_BATCH: usize = 32;
    if runtime.buffer.len() >= FLUSH_BATCH {
        flush_remote_buffer(runtime).await;
    }
}

async fn telemetry_worker(data_root: PathBuf, mut rx: mpsc::Receiver<TelemetryCommand>) {
    let mut runtime = TelemetryRuntime {
        cfg: TelemetryConfig::default(),
        install_id: None,
        buffer: Vec::new(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        client: reqwest::Client::builder()
            .timeout(TELEMETRY_REQUEST_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new()),
    };

    let mut flush_tick = tokio::time::interval(Duration::from_secs(10));

    loop {
        tokio::select! {
            _ = flush_tick.tick() => {
                flush_remote_buffer(&mut runtime).await;
            }
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    TelemetryCommand::Event(event) => {
                        process_event(&mut runtime, &data_root, event).await;
                    }
                    TelemetryCommand::Events(events) => {
                        for event in events {
                            process_event(&mut runtime, &data_root, event).await;
                        }
                    }
                    TelemetryCommand::UpdateConfig(cfg) => {
                        runtime.cfg = cfg;
                        if runtime.cfg.enabled {
                            let _ = ensure_broker_install_id(&mut runtime, &data_root).await;
                        } else {
                            runtime.buffer.clear();
                        }
                    }
                    TelemetryCommand::Flush(done) => {
                        flush_remote_buffer(&mut runtime).await;
                        let _ = done.send(());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TelemetryDelivery, TelemetryEvent, TelemetryOriginRuntime, TelemetryPlane};

    #[test]
    fn session_interrupt_latency_event_sets_bounded_fields() {
        let event = TelemetryEvent::session_interrupt_latency(
            "codex".to_string(),
            "gpt-5.2-codex".to_string(),
            Some("host".to_string()),
            Some("worktree".to_string()),
            1320,
            "1s_to_3s".to_string(),
        );

        assert_eq!(event.event_name, "session_interrupt_latency");
        assert_eq!(event.plane, TelemetryPlane::Product);
        assert_eq!(event.delivery, TelemetryDelivery::Remote);
        assert_eq!(event.origin_runtime, TelemetryOriginRuntime::Daemon);
        assert_eq!(
            event.properties.get("duration_ms"),
            Some(&serde_json::json!(1320))
        );
        assert_eq!(
            event.properties.get("duration_bucket"),
            Some(&serde_json::json!("1s_to_3s"))
        );
        assert_eq!(
            event.properties.get("status"),
            Some(&serde_json::json!("interrupted"))
        );
        assert_eq!(
            event.properties.get("success"),
            Some(&serde_json::json!(true))
        );
    }

    #[test]
    fn local_only_marks_delivery_without_mutating_plane() {
        let event = TelemetryEvent::daemon_incident("renderer_backlog_sample").local_only();
        assert_eq!(event.event_name, "renderer_backlog_sample");
        assert_eq!(event.plane, TelemetryPlane::Incident);
        assert_eq!(event.delivery, TelemetryDelivery::LocalOnly);
    }
}
