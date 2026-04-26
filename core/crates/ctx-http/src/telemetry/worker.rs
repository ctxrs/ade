use super::*;

#[derive(Debug)]
pub(super) enum TelemetryCommand {
    Event(Box<TelemetryEvent>),
    Events(Vec<TelemetryEvent>),
    UpdateConfig(TelemetryConfig),
    Flush(oneshot::Sender<()>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TelemetryStateFile {
    pub(super) install_id: String,
    pub(super) created_at: DateTime<Utc>,
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

pub(super) fn telemetry_state_path(data_root: &Path) -> PathBuf {
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

pub(super) async fn load_or_create_install_id(data_root: &Path) -> Option<String> {
    let path = telemetry_state_path(data_root);
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => match serde_json::from_str::<TelemetryStateFile>(&raw) {
            Ok(state) if !state.install_id.trim().is_empty() => return Some(state.install_id),
            Ok(_) => {
                tracing::warn!("telemetry install id missing from {}", path.display());
                return None;
            }
            Err(err) => {
                tracing::warn!(
                    "failed to parse telemetry state {}: {err:#}",
                    path.display()
                );
                return None;
            }
        },
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            tracing::warn!("failed to read telemetry state {}: {err:#}", path.display());
            return None;
        }
        Err(_) => {}
    }

    let install_id = uuid::Uuid::new_v4().to_string();
    let state = TelemetryStateFile {
        install_id: install_id.clone(),
        created_at: Utc::now(),
    };
    let Some(parent) = path.parent() else {
        tracing::warn!("telemetry state path has no parent: {}", path.display());
        return None;
    };
    if let Err(err) = tokio::fs::create_dir_all(parent).await {
        tracing::warn!(
            "failed to create telemetry state directory {}: {err:#}",
            parent.display()
        );
        return None;
    }
    let Ok(bytes) = serde_json::to_vec_pretty(&state) else {
        tracing::warn!("failed to serialize telemetry state for {}", path.display());
        return None;
    };
    if let Err(err) = tokio::fs::write(&path, bytes).await {
        tracing::warn!(
            "failed to write telemetry state {}: {err:#}",
            path.display()
        );
        return None;
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

pub(super) async fn telemetry_worker(data_root: PathBuf, mut rx: mpsc::Receiver<TelemetryCommand>) {
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
                        process_event(&mut runtime, &data_root, *event).await;
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
