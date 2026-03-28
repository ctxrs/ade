use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::time::MissedTickBehavior;

use ctx_providers::adapters::ProviderProcessInfo;

use crate::daemon::AppState;
use crate::logs;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind, PerfTelemetry};
use crate::resource_utilization::{
    ProviderMemoryRollup, ResourceProcess, ResourceProcesses, SystemSnapshot,
};
use crate::workspace_runtime::SubstrateLifecycleRecord;

const RESOURCE_LOG_PREFIX: &str = "resource-util-";
const RESOURCE_LOG_SUFFIX: &str = ".jsonl";

const DEFAULT_INTERVAL_MS: u64 = 15_000;
const DEFAULT_RETENTION_DAYS: u64 = 7;
const DEFAULT_MAX_BYTES: u64 = 25 * 1024 * 1024;
const DEFAULT_CHILD_LIMIT: usize = 10;

#[derive(Debug, Clone)]
struct ResourceTelemetryConfig {
    interval: Duration,
    local_retention_days: u64,
    local_max_bytes: u64,
    child_limit: usize,
}

impl ResourceTelemetryConfig {
    fn from_env() -> Self {
        let interval_ms =
            env_u64("CTX_RESOURCE_TELEMETRY_INTERVAL_MS").unwrap_or(DEFAULT_INTERVAL_MS);
        let local_retention_days = env_u64("CTX_RESOURCE_TELEMETRY_LOCAL_RETENTION_DAYS")
            .unwrap_or(DEFAULT_RETENTION_DAYS);
        let local_max_bytes =
            env_u64("CTX_RESOURCE_TELEMETRY_LOCAL_MAX_BYTES").unwrap_or(DEFAULT_MAX_BYTES);
        let child_limit = env_u64("CTX_RESOURCE_TELEMETRY_CHILD_LIMIT")
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_CHILD_LIMIT);

        Self {
            interval: Duration::from_millis(interval_ms),
            local_retention_days,
            local_max_bytes,
            child_limit,
        }
    }

    fn enabled(&self) -> bool {
        !self.interval.is_zero()
    }
}

#[derive(Debug, Serialize)]
struct ResourceTelemetryEvent {
    occurred_at: DateTime<Utc>,
    cache_age_ms: u64,
    system: SystemSnapshot,
    processes: ResourceProcesses,
    provider_sessions: HashMap<String, u64>,
    provider_memory_rollups: Vec<ProviderMemoryRollup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shared_substrate_lifecycle: Option<SubstrateLifecycleRecord>,
}

pub fn spawn_resource_telemetry(state: Arc<AppState>) {
    if resource_utilization_disabled() {
        return;
    }
    let cfg = ResourceTelemetryConfig::from_env();
    if !cfg.enabled() {
        return;
    }

    let mut shutdown_rx = state.core.shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut last_cleanup = None::<String>;
        if let Err(err) = sample_once(&state, &cfg, &mut last_cleanup).await {
            tracing::warn!("resource telemetry sample failed: {err:#}");
        }

        let mut ticker = tokio::time::interval(cfg.interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                _ = ticker.tick() => {
                    if let Err(err) = sample_once(&state, &cfg, &mut last_cleanup).await {
                        tracing::warn!("resource telemetry sample failed: {err:#}");
                    }
                }
            }
        }
    });
}

async fn sample_once(
    state: &Arc<AppState>,
    cfg: &ResourceTelemetryConfig,
    last_cleanup: &mut Option<String>,
) -> Result<()> {
    let provider_processes = list_provider_processes(state).await;
    let (system, cache_age_ms, processes, provider_memory_rollups) = {
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        let (system, _disks, cache_age_ms) = sampler.system_snapshot();
        let processes = sampler.processes_snapshot_light(std::process::id(), &provider_processes);
        let provider_memory_rollups = sampler.provider_memory_rollups(&provider_processes);
        (system, cache_age_ms, processes, provider_memory_rollups)
    };

    let provider_sessions = provider_session_counts(state).await;
    let shared_substrate_lifecycle =
        crate::workspace_runtime::selected_shared_substrate_lifecycle(&state.core.data_root)
            .ok()
            .flatten();
    let processes = trim_processes(processes, cfg.child_limit);
    let event = ResourceTelemetryEvent {
        occurred_at: Utc::now(),
        cache_age_ms,
        system: system.clone(),
        processes: processes.clone(),
        provider_sessions: provider_sessions.clone(),
        provider_memory_rollups,
        shared_substrate_lifecycle: shared_substrate_lifecycle.clone(),
    };

    append_local_log(&state.core.data_root, &event, cfg).await?;
    if cfg.local_retention_days > 0 {
        let today = event.occurred_at.format("%Y-%m-%d").to_string();
        if last_cleanup.as_deref() != Some(&today) {
            let _ = cleanup_old_logs(&state.core.data_root, cfg.local_retention_days).await;
            *last_cleanup = Some(today);
        }
    }

    export_remote_metrics(
        &state.telemetry.perf_telemetry,
        &system,
        &processes,
        &provider_sessions,
        shared_substrate_lifecycle.as_ref(),
    );
    Ok(())
}

async fn list_provider_processes(state: &Arc<AppState>) -> Vec<ProviderProcessInfo> {
    let providers = {
        let providers = state.providers.adapters.lock().await;
        providers.values().cloned().collect::<Vec<_>>()
    };
    let mut processes = Vec::new();
    for adapter in providers {
        processes.extend(adapter.list_processes().await);
    }
    processes
}

async fn provider_session_counts(state: &Arc<AppState>) -> HashMap<String, u64> {
    let session_ids = state.list_running_sessions().await;
    let mut counts: HashMap<String, u64> = HashMap::new();
    for session_id in session_ids {
        let session = match state.store_for_session(session_id).await {
            Ok(store) => store.get_session(session_id).await.ok().flatten(),
            Err(_) => None,
        };
        let Some(session) = session else {
            continue;
        };
        *counts.entry(session.provider_id).or_insert(0) += 1;
    }
    counts
}

fn trim_processes(processes: ResourceProcesses, child_limit: usize) -> ResourceProcesses {
    ResourceProcesses {
        daemon: processes.daemon.map(|proc| trim_process(proc, child_limit)),
        providers: processes
            .providers
            .into_iter()
            .map(|proc| trim_process(proc, child_limit))
            .collect(),
    }
}

fn trim_process(mut proc: ResourceProcess, child_limit: usize) -> ResourceProcess {
    if proc.children.is_empty() {
        return proc;
    }

    if child_limit == 0 {
        proc.children.clear();
        proc.children_truncated = true;
        return proc;
    }

    proc.children.sort_by(|a, b| {
        b.cpu_pct
            .total_cmp(&a.cpu_pct)
            .then_with(|| b.memory_bytes.cmp(&a.memory_bytes))
            .then_with(|| a.pid.cmp(&b.pid))
    });

    if proc.children.len() > child_limit {
        proc.children.truncate(child_limit);
        proc.children_truncated = true;
    } else if proc.child_count as usize > proc.children.len() {
        proc.children_truncated = true;
    }

    proc
}

fn export_remote_metrics(
    perf: &PerfTelemetry,
    system: &SystemSnapshot,
    processes: &ResourceProcesses,
    provider_sessions: &HashMap<String, u64>,
    shared_substrate_lifecycle: Option<&SubstrateLifecycleRecord>,
) {
    let labels = HashMap::new();
    perf.export_remote_metric(PerfMetric {
        name: "ctx.system.cpu_pct".to_string(),
        kind: PerfMetricKind::Gauge,
        unit: "percent".to_string(),
        value: system.cpu_pct as f64,
        labels: labels.clone(),
    });
    perf.export_remote_metric(PerfMetric {
        name: "ctx.system.mem_used_bytes".to_string(),
        kind: PerfMetricKind::Gauge,
        unit: "bytes".to_string(),
        value: system.memory_used_bytes as f64,
        labels: labels.clone(),
    });
    perf.export_remote_metric(PerfMetric {
        name: "ctx.system.swap_used_bytes".to_string(),
        kind: PerfMetricKind::Gauge,
        unit: "bytes".to_string(),
        value: system.swap_used_bytes as f64,
        labels: labels.clone(),
    });

    if let Some(daemon) = processes.daemon.as_ref() {
        perf.export_remote_metric(PerfMetric {
            name: "ctx.daemon.cpu_pct".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "percent".to_string(),
            value: daemon.cpu_pct as f64,
            labels: labels.clone(),
        });
        perf.export_remote_metric(PerfMetric {
            name: "ctx.daemon.mem_bytes".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "bytes".to_string(),
            value: daemon.memory_bytes as f64,
            labels: labels.clone(),
        });
    }

    let mut provider_agg: HashMap<String, ProviderAggregate> = HashMap::new();
    for proc in processes.providers.iter() {
        let label = proc.label.clone();
        let entry = provider_agg.entry(label).or_default();
        entry.cpu_pct += proc.cpu_pct as f64;
        entry.mem_bytes += proc.memory_bytes;
        entry.process_count += 1;
    }

    for (provider_id, agg) in provider_agg.iter() {
        let mut labels = HashMap::new();
        labels.insert("provider_id".to_string(), provider_id.clone());
        perf.export_remote_metric(PerfMetric {
            name: "ctx.provider.cpu_pct".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "percent".to_string(),
            value: agg.cpu_pct,
            labels: labels.clone(),
        });
        perf.export_remote_metric(PerfMetric {
            name: "ctx.provider.mem_bytes".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "bytes".to_string(),
            value: agg.mem_bytes as f64,
            labels: labels.clone(),
        });
        perf.export_remote_metric(PerfMetric {
            name: "ctx.provider.process_count".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "count".to_string(),
            value: agg.process_count as f64,
            labels: labels.clone(),
        });
    }

    for (provider_id, count) in provider_sessions.iter() {
        let mut labels = HashMap::new();
        labels.insert("provider_id".to_string(), provider_id.clone());
        perf.export_remote_metric(PerfMetric {
            name: "ctx.provider.session_count".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "count".to_string(),
            value: *count as f64,
            labels,
        });
    }

    if let Some(record) = shared_substrate_lifecycle {
        let mut base_labels = HashMap::new();
        if let Some(substrate) = serde_label(&record.substrate) {
            base_labels.insert("substrate_kind".to_string(), substrate);
        }

        perf.export_remote_metric(PerfMetric {
            name: "ctx.substrate.simulated".to_string(),
            kind: PerfMetricKind::Gauge,
            unit: "bool".to_string(),
            value: if record.simulated { 1.0 } else { 0.0 },
            labels: base_labels.clone(),
        });

        if let Some(startup_selection) = record.startup_selection.as_ref().and_then(serde_label) {
            let mut labels = base_labels.clone();
            labels.insert("startup_selection".to_string(), startup_selection);
            perf.export_remote_metric(PerfMetric {
                name: "ctx.substrate.startup_selection".to_string(),
                kind: PerfMetricKind::Gauge,
                unit: "state".to_string(),
                value: 1.0,
                labels,
            });
        }

        if let Some(startup_outcome) = record.startup_outcome.as_ref().and_then(serde_label) {
            let mut labels = base_labels.clone();
            labels.insert("startup_outcome".to_string(), startup_outcome);
            if let Some(startup_reason) = record.startup_reason.as_ref().and_then(serde_label) {
                labels.insert("startup_reason".to_string(), startup_reason);
            }
            perf.export_remote_metric(PerfMetric {
                name: "ctx.substrate.startup_outcome".to_string(),
                kind: PerfMetricKind::Gauge,
                unit: "state".to_string(),
                value: 1.0,
                labels,
            });
        }

        if let Some(shutdown_outcome) = record.shutdown_outcome.as_ref().and_then(serde_label) {
            let mut labels = base_labels;
            labels.insert("shutdown_outcome".to_string(), shutdown_outcome);
            if let Some(shutdown_reason) = record.shutdown_reason.as_ref().and_then(serde_label) {
                labels.insert("shutdown_reason".to_string(), shutdown_reason);
            }
            perf.export_remote_metric(PerfMetric {
                name: "ctx.substrate.shutdown_outcome".to_string(),
                kind: PerfMetricKind::Gauge,
                unit: "state".to_string(),
                value: 1.0,
                labels,
            });
        }
    }
}

#[derive(Default)]
struct ProviderAggregate {
    cpu_pct: f64,
    mem_bytes: u64,
    process_count: u64,
}

fn serde_label<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_value(value)
        .ok()?
        .as_str()
        .map(ToString::to_string)
}

async fn append_local_log(
    data_root: &Path,
    event: &ResourceTelemetryEvent,
    cfg: &ResourceTelemetryConfig,
) -> Result<()> {
    let dir = logs::logs_dir(data_root);
    tokio::fs::create_dir_all(&dir).await.ok();
    let date = event.occurred_at.format("%Y-%m-%d").to_string();
    let path = dir.join(format!(
        "{}{}{}",
        RESOURCE_LOG_PREFIX, date, RESOURCE_LOG_SUFFIX
    ));

    if cfg.local_max_bytes > 0 {
        if let Ok(metadata) = tokio::fs::metadata(&path).await {
            if metadata.len() >= cfg.local_max_bytes {
                return Ok(());
            }
        }
    }

    let line = serde_json::to_string(event)?;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await?;
    use tokio::io::AsyncWriteExt;
    file.write_all(line.as_bytes()).await?;
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
        if !file_name.starts_with(RESOURCE_LOG_PREFIX) || !file_name.ends_with(RESOURCE_LOG_SUFFIX)
        {
            continue;
        }
        let date = file_name
            .trim_start_matches(RESOURCE_LOG_PREFIX)
            .trim_end_matches(RESOURCE_LOG_SUFFIX);
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

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
}

fn resource_utilization_disabled() -> bool {
    env_bool("CTX_RESOURCE_UTILIZATION_DISABLED").unwrap_or(true)
}

fn env_bool(key: &str) -> Option<bool> {
    std::env::var(key)
        .ok()
        .as_deref()
        .and_then(ctx_core::boolish::parse_boolish)
}
