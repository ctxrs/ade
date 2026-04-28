use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use ctx_providers::adapters::ProviderUnknownEventHook;
use ctx_providers::events::ProviderUnknownEventObservation;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::logs;
use crate::telemetry::{Telemetry, TelemetryEvent, TelemetryProperties};

const EVENT_NAME: &str = "provider_unknown_event_observed";
const LOG_DIR_NAME: &str = "provider-unknown-events";
const LOG_PREFIX: &str = "provider-unknown-events-";
const LOG_SUFFIX: &str = ".jsonl";
const DEFAULT_SAMPLE_RATE: f64 = 0.01;
const DEFAULT_LOCAL_MAX_BYTES: u64 = 50 * 1024 * 1024;
const DEFAULT_LOCAL_RETENTION_DAYS: u64 = 7;
const CHANNEL_SEND_TIMEOUT: Duration = Duration::from_millis(100);
const MAX_DIAGNOSTIC_LINE_BYTES: usize = 16 * 1024;
const MAX_REMOTE_STRING_CHARS: usize = 96;

#[derive(Clone, Debug)]
pub(crate) struct ProviderUnknownEventContext {
    pub(crate) provider_id: String,
    pub(crate) execution_environment: Option<String>,
    pub(crate) session_root_kind: Option<String>,
    pub(crate) operation: String,
}

#[derive(Clone)]
pub(crate) struct ProviderUnknownEvents {
    tx: mpsc::Sender<ProviderUnknownEventCommand>,
}

struct ProviderUnknownEventCommand {
    context: ProviderUnknownEventContext,
    observation: ProviderUnknownEventObservation,
    sample_rate: f64,
}

#[derive(Debug, Clone, Copy)]
struct LocalLogConfig {
    max_bytes: u64,
    retention_days: u64,
}

impl ProviderUnknownEvents {
    pub(crate) fn new(data_root: PathBuf, telemetry: Telemetry) -> Self {
        let (tx, rx) = mpsc::channel(512);
        tokio::spawn(async move {
            provider_unknown_events_worker(data_root, telemetry, rx).await;
        });
        Self { tx }
    }

    pub(crate) async fn observe(
        &self,
        context: ProviderUnknownEventContext,
        observation: ProviderUnknownEventObservation,
    ) {
        let sample_rate = configured_sample_rate();
        if !should_sample(&context, &observation, sample_rate) {
            return;
        }
        let command = ProviderUnknownEventCommand {
            context,
            observation,
            sample_rate,
        };
        match timeout(CHANNEL_SEND_TIMEOUT, self.tx.send(command)).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => tracing::warn!("provider unknown event channel closed; dropping sample"),
            Err(_) => tracing::warn!("provider unknown event channel blocked; dropping sample"),
        }
    }
}

pub(crate) fn provider_unknown_event_hook(
    sink: ProviderUnknownEvents,
    context: ProviderUnknownEventContext,
) -> ProviderUnknownEventHook {
    Arc::new(move |observation| {
        let sink = sink.clone();
        let context = context.clone();
        Box::pin(async move {
            sink.observe(context, observation).await;
        })
    })
}

async fn provider_unknown_events_worker(
    data_root: PathBuf,
    telemetry: Telemetry,
    mut rx: mpsc::Receiver<ProviderUnknownEventCommand>,
) {
    while let Some(command) = rx.recv().await {
        if let Err(err) = process_provider_unknown_event(&data_root, &telemetry, command).await {
            tracing::warn!("failed to process provider unknown event sample: {err:#}");
        }
    }
}

async fn process_provider_unknown_event(
    data_root: &Path,
    telemetry: &Telemetry,
    command: ProviderUnknownEventCommand,
) -> Result<()> {
    let log_config = local_log_config_from_env();
    let line = diagnostic_log_line(&command.context, &command.observation)?;
    if let Err(err) = append_local_diagnostic_line(data_root, &line, log_config).await {
        tracing::warn!("failed to append provider unknown event diagnostic sample: {err:#}");
    }
    telemetry
        .emit(build_telemetry_event(
            &command.context,
            &command.observation,
            command.sample_rate,
        ))
        .await;
    Ok(())
}

fn diagnostic_log_line(
    context: &ProviderUnknownEventContext,
    observation: &ProviderUnknownEventObservation,
) -> Result<String> {
    let raw = redact_diagnostic_value(&observation.raw);
    let mut payload = diagnostic_payload(context, observation, raw, false);
    let mut line = serde_json::to_string(&payload)?;
    if line.len() <= MAX_DIAGNOSTIC_LINE_BYTES {
        return Ok(line);
    }

    payload = diagnostic_payload(
        context,
        observation,
        json!("[omitted: diagnostic sample exceeded line limit]"),
        true,
    );
    line = serde_json::to_string(&payload)?;
    Ok(line)
}

fn diagnostic_payload(
    context: &ProviderUnknownEventContext,
    observation: &ProviderUnknownEventObservation,
    raw: Value,
    raw_omitted: bool,
) -> Value {
    json!({
        "captured_at": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        "kind": EVENT_NAME,
        "provider_id": context.provider_id,
        "operation": context.operation,
        "execution_environment": context.execution_environment,
        "session_root_kind": context.session_root_kind,
        "protocol": observation.protocol,
        "event_type": observation.event_type,
        "event_type_hash": stable_hash(&observation.event_type),
        "parse_error": truncate_string(&logs::redact_sensitive(&observation.parse_error), 512),
        "parse_error_kind": classify_parse_error(&observation.parse_error),
        "crp_channel": observation.crp_channel,
        "crp_seq": observation.crp_seq,
        "timeline_notice_emitted": observation.timeline_notice_emitted,
        "raw_truncated": observation.raw_truncated,
        "raw_omitted": raw_omitted,
        "raw": raw,
    })
}

async fn append_local_diagnostic_line(
    data_root: &Path,
    line: &str,
    config: LocalLogConfig,
) -> Result<()> {
    let dir = logs::logs_dir(data_root).join(LOG_DIR_NAME);
    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("creating provider unknown event log dir {}", dir.display()))?;
    cleanup_local_diagnostic_logs(&dir, config).await?;

    let path = local_log_path_for_now(&dir);
    let line_bytes = line.len() as u64 + 1;
    if config.max_bytes > 0 {
        if let Ok(metadata) = tokio::fs::metadata(&path).await {
            if metadata.len().saturating_add(line_bytes) > config.max_bytes {
                let _ = tokio::fs::remove_file(&path).await;
            }
        }
    }

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .with_context(|| format!("opening provider unknown event log {}", path.display()))?;
    use tokio::io::AsyncWriteExt;
    file.write_all(line.as_bytes()).await?;
    file.write_all(b"\n").await?;
    file.flush().await?;

    cleanup_local_diagnostic_logs(&dir, config).await?;
    Ok(())
}

fn local_log_path_for_now(dir: &Path) -> PathBuf {
    dir.join(format!(
        "{LOG_PREFIX}{}{LOG_SUFFIX}",
        Utc::now().format("%Y-%m-%d")
    ))
}

async fn cleanup_local_diagnostic_logs(dir: &Path, config: LocalLogConfig) -> Result<()> {
    let mut entries = local_diagnostic_log_entries(dir).await?;
    if config.retention_days > 0 {
        let cutoff = Utc::now() - chrono::Duration::days(config.retention_days as i64);
        let mut retained = Vec::with_capacity(entries.len());
        for entry in entries {
            if entry.modified < cutoff {
                let _ = tokio::fs::remove_file(&entry.path).await;
            } else {
                retained.push(entry);
            }
        }
        entries = retained;
    }

    if config.max_bytes == 0 {
        return Ok(());
    }
    entries.sort_by_key(|entry| entry.modified);
    let mut total = entries.iter().map(|entry| entry.bytes).sum::<u64>();
    for entry in entries {
        if total <= config.max_bytes {
            break;
        }
        if tokio::fs::remove_file(&entry.path).await.is_ok() {
            total = total.saturating_sub(entry.bytes);
        }
    }
    Ok(())
}

#[derive(Debug)]
struct LocalLogEntry {
    path: PathBuf,
    bytes: u64,
    modified: DateTime<Utc>,
}

async fn local_diagnostic_log_entries(dir: &Path) -> Result<Vec<LocalLogEntry>> {
    let mut out = Vec::new();
    let mut rd = match tokio::fs::read_dir(dir).await {
        Ok(rd) => rd,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(err) => return Err(err).context("reading provider unknown event log dir"),
    };
    while let Some(entry) = rd.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(LOG_PREFIX) || !name.ends_with(LOG_SUFFIX) {
            continue;
        }
        let metadata = entry.metadata().await?;
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        out.push(LocalLogEntry {
            path: entry.path(),
            bytes: metadata.len(),
            modified: DateTime::<Utc>::from(modified),
        });
    }
    Ok(out)
}

fn build_telemetry_event(
    context: &ProviderUnknownEventContext,
    observation: &ProviderUnknownEventObservation,
    sample_rate: f64,
) -> TelemetryEvent {
    let mut properties = TelemetryProperties::new();
    insert_string(&mut properties, "provider_id", &context.provider_id);
    insert_string(&mut properties, "operation", &context.operation);
    if let Some(value) = context.execution_environment.as_deref() {
        insert_string(&mut properties, "execution_environment", value);
    }
    if let Some(value) = context.session_root_kind.as_deref() {
        insert_string(&mut properties, "session_root_kind", value);
    }
    insert_string(&mut properties, "protocol", observation.protocol);
    let event_type_safe = safe_remote_event_type(&observation.event_type);
    properties.insert("event_type_safe".to_string(), json!(event_type_safe));
    properties.insert(
        "event_type_length".to_string(),
        json!(observation.event_type.len()),
    );
    if event_type_safe {
        insert_string(&mut properties, "event_type", &observation.event_type);
        properties.insert(
            "event_type_hash".to_string(),
            json!(stable_hash(&observation.event_type)),
        );
    }
    insert_string(
        &mut properties,
        "parse_error_kind",
        classify_parse_error(&observation.parse_error),
    );
    if let Some(channel) = observation.crp_channel.as_deref() {
        insert_string(&mut properties, "crp_channel", channel);
    }
    properties.insert(
        "timeline_notice_emitted".to_string(),
        json!(observation.timeline_notice_emitted),
    );
    properties.insert(
        "diagnostic_only".to_string(),
        json!(!observation.timeline_notice_emitted),
    );
    properties.insert(
        "raw_truncated".to_string(),
        json!(observation.raw_truncated),
    );
    properties.insert(
        "raw_bytes".to_string(),
        json!(raw_size_bytes(&observation.raw)),
    );
    properties.insert(
        "raw_top_level_keys".to_string(),
        json!(observation.raw.as_object().map_or(0, Map::len)),
    );
    properties.insert("sample_rate".to_string(), json!(sample_rate));

    TelemetryEvent::daemon_incident(EVENT_NAME)
        .with_source("provider_unknown_events")
        .with_properties(properties)
}

fn insert_string(properties: &mut TelemetryProperties, key: &str, value: &str) {
    properties.insert(
        key.to_string(),
        json!(truncate_string(value, MAX_REMOTE_STRING_CHARS)),
    );
}

fn configured_sample_rate() -> f64 {
    std::env::var("CTX_PROVIDER_UNKNOWN_EVENT_SAMPLE_RATE")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(0.0, 1.0))
        .unwrap_or(DEFAULT_SAMPLE_RATE)
}

fn local_log_config_from_env() -> LocalLogConfig {
    LocalLogConfig {
        max_bytes: env_u64(
            "CTX_PROVIDER_UNKNOWN_EVENT_LOCAL_MAX_BYTES",
            DEFAULT_LOCAL_MAX_BYTES,
        ),
        retention_days: env_u64(
            "CTX_PROVIDER_UNKNOWN_EVENT_LOCAL_RETENTION_DAYS",
            DEFAULT_LOCAL_RETENTION_DAYS,
        ),
    }
}

fn env_u64(key: &str, default_value: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default_value)
}

fn should_sample(
    context: &ProviderUnknownEventContext,
    observation: &ProviderUnknownEventObservation,
    sample_rate: f64,
) -> bool {
    if sample_rate <= 0.0 {
        return false;
    }
    if sample_rate >= 1.0 {
        return true;
    }
    let mut hasher = Sha256::new();
    hasher.update(context.provider_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(context.operation.as_bytes());
    hasher.update(b"\0");
    hasher.update(observation.event_type.as_bytes());
    hasher.update(b"\0");
    hasher.update(observation.crp_seq.to_be_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    let value = u64::from_be_bytes(bytes);
    let threshold = (sample_rate * u64::MAX as f64) as u64;
    value <= threshold
}

fn stable_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(&digest[..8])
}

fn raw_size_bytes(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(0, |bytes| bytes.len())
}

fn truncate_string(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn classify_parse_error(parse_error: &str) -> &'static str {
    let lower = parse_error.to_ascii_lowercase();
    if lower.contains("unknown variant") {
        "unknown_variant"
    } else if lower.contains("missing field") {
        "missing_field"
    } else if lower.contains("invalid type") {
        "invalid_type"
    } else if lower.contains("invalid value") {
        "invalid_value"
    } else {
        "schema_error"
    }
}

fn safe_remote_event_type(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= MAX_REMOTE_STRING_CHARS
        && trimmed.split('.').all(|segment| {
            let mut chars = segment.chars();
            chars.next().is_some_and(|first| first.is_ascii_lowercase())
                && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        })
}

fn redact_diagnostic_value(value: &Value) -> Value {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(value) => Value::String(logs::redact_sensitive(value)),
        Value::Array(items) => Value::Array(items.iter().map(redact_diagnostic_value).collect()),
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (key, value) in map {
                if sensitive_json_key(key) {
                    out.insert(key.clone(), json!("[REDACTED]"));
                } else {
                    out.insert(key.clone(), redact_diagnostic_value(value));
                }
            }
            Value::Object(out)
        }
    }
}

fn sensitive_json_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    [
        "token",
        "secret",
        "apikey",
        "authorization",
        "cookie",
        "password",
        "credential",
        "oauth",
        "authurl",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

#[cfg(test)]
mod tests;
