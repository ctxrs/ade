use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde_json::json;
use tokio::sync::{broadcast, Mutex};

use ctx_core::models::SessionEventType;

use crate::daemon::AppState;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};
use ctx_settings_model::{ProviderGuardSettings, ResourceGovernanceMode, Settings};

pub use ctx_provider_runtime::provider_guard::{
    compute_effective_limits, ProviderGuardConfig, ProviderGuardLimits, ProviderGuardRuntime,
    ResourceGovernanceMode as GuardMode,
};

pub async fn apply_settings(state: &AppState, settings: &Settings) -> Result<()> {
    let cfg = settings.provider_guard.clone().unwrap_or_default();
    ctx_provider_runtime::provider_guard::apply_settings(state, &map_config(&cfg)).await
}

pub fn spawn_provider_guard(state: Arc<AppState>) {
    ctx_provider_runtime::provider_guard::spawn_provider_guard(state);
}

#[async_trait::async_trait]
impl ctx_provider_runtime::provider_guard::ProviderGuardHost for AppState {
    fn provider_guard_runtime(&self) -> &Mutex<ProviderGuardRuntime> {
        &self.providers.guard
    }

    fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.core.shutdown_tx.subscribe()
    }

    async fn system_snapshot(&self) -> ctx_provider_runtime::provider_guard::SystemSnapshot {
        let (system, _disks, _cache_age_ms) = {
            let mut sampler = self.telemetry.resource_sampler.lock().await;
            sampler.system_snapshot()
        };
        ctx_provider_runtime::provider_guard::SystemSnapshot {
            memory_total_bytes: system.memory_total_bytes,
            memory_used_bytes: system.memory_used_bytes,
        }
    }

    async fn provider_memory_snapshot(
        &self,
    ) -> Vec<ctx_provider_runtime::provider_guard::ProviderMemorySample> {
        let provider_processes = list_provider_processes(self).await;
        let samples = {
            let mut sampler = self.telemetry.resource_sampler.lock().await;
            sampler.provider_memory_snapshot(&provider_processes)
        };
        samples
            .into_iter()
            .map(
                |sample| ctx_provider_runtime::provider_guard::ProviderMemorySample {
                    provider_id: sample.provider_id,
                    label: sample.label,
                    pid: sample.pid,
                    memory_bytes: sample.memory_bytes,
                    tool_memory_bytes: sample.tool_memory_bytes,
                },
            )
            .collect()
    }

    async fn on_provider_guard_event(
        state: &Arc<Self>,
        event: ctx_provider_runtime::provider_guard::ProviderGuardEvent,
    ) {
        log_guard_event(state, &event).await;
        if event.stage == "max" || event.stage == "kill" {
            capture_guard_snapshot(state, &event).await;
        }
        notify_sessions(state, &event).await;
    }
}

fn map_config(settings: &ProviderGuardSettings) -> ProviderGuardConfig {
    ProviderGuardConfig {
        enabled: settings.enabled,
        mode: Some(match settings.mode {
            ResourceGovernanceMode::Auto => GuardMode::Auto,
            ResourceGovernanceMode::Custom => GuardMode::Custom,
        }),
        memory_high_mb: settings.memory_high_mb,
        memory_max_mb: settings.memory_max_mb,
        interval_ms: settings.interval_ms,
        grace_period_ms: settings.grace_period_ms,
    }
}

async fn log_guard_event(
    state: &AppState,
    event: &ctx_provider_runtime::provider_guard::ProviderGuardEvent,
) {
    let mem_mb = bytes_to_mb(event.sample.memory_bytes);
    tracing::warn!(
        provider_id = %event.sample.label,
        pid = event.sample.pid,
        event = event.stage,
        memory_mb = mem_mb,
        limit_high_mb = event.limits.memory_high_mb,
        limit_max_mb = event.limits.memory_max_mb,
        "provider guard triggered"
    );

    let mut labels = HashMap::new();
    labels.insert("provider_id".to_string(), event.sample.label.clone());
    labels.insert("event".to_string(), event.stage.to_string());
    state
        .telemetry
        .perf_telemetry
        .record_metric(
            PerfMetric {
                name: "ctx.provider.guard.events".to_string(),
                kind: PerfMetricKind::Counter,
                unit: "count".to_string(),
                value: 1.0,
                labels,
            },
            None,
            None,
            None,
        )
        .await;
}

async fn capture_guard_snapshot(
    state: &AppState,
    event: &ctx_provider_runtime::provider_guard::ProviderGuardEvent,
) {
    #[cfg(target_os = "linux")]
    {
        let timestamp_ms = unix_ms_now();
        let dir = state.core.data_root.join("logs").join("providers");
        let path = dir.join(format!(
            "provider-guard-{}-{}-{}-{}.log",
            event.sample.label, event.sample.pid, event.stage, timestamp_ms
        ));
        if tokio::fs::create_dir_all(&dir).await.is_err() {
            return;
        }

        let mut output = String::new();
        output.push_str(&format!("event={}\n", event.stage));
        output.push_str(&format!("pid={}\n", event.sample.pid));
        output.push_str(&format!("label={}\n", event.sample.label));
        output.push_str(&format!("memory_bytes={}\n", event.sample.memory_bytes));
        output.push_str(&format!("timestamp_ms={timestamp_ms}\n\n"));

        let status_path = format!("/proc/{}/status", event.sample.pid);
        match tokio::fs::read_to_string(&status_path).await {
            Ok(status) => {
                output.push_str("== /proc/pid/status ==\n");
                output.push_str(&status);
                output.push('\n');
            }
            Err(err) => {
                output.push_str("== /proc/pid/status ==\n");
                output.push_str(&format!("error={err:#}\n\n"));
            }
        }

        let smaps_path = format!("/proc/{}/smaps_rollup", event.sample.pid);
        match tokio::fs::read_to_string(&smaps_path).await {
            Ok(smaps) => {
                output.push_str("== /proc/pid/smaps_rollup ==\n");
                output.push_str(&smaps);
                output.push('\n');
            }
            Err(err) => {
                output.push_str("== /proc/pid/smaps_rollup ==\n");
                output.push_str(&format!("error={err:#}\n\n"));
            }
        }

        let cmdline_path = format!("/proc/{}/cmdline", event.sample.pid);
        match tokio::fs::read(&cmdline_path).await {
            Ok(cmdline) => {
                let printable = cmdline
                    .split(|b| *b == 0)
                    .filter_map(|part| std::str::from_utf8(part).ok())
                    .collect::<Vec<_>>()
                    .join(" ");
                output.push_str("== /proc/pid/cmdline ==\n");
                output.push_str(&printable);
                output.push('\n');
            }
            Err(err) => {
                output.push_str("== /proc/pid/cmdline ==\n");
                output.push_str(&format!("error={err:#}\n"));
            }
        }

        if tokio::fs::write(&path, output).await.is_ok() {
            tracing::info!(
                provider_id = %event.sample.label,
                pid = event.sample.pid,
                path = %path.display(),
                "captured provider guard snapshot"
            );
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (state, event);
    }
}

async fn notify_sessions(
    state: &Arc<AppState>,
    event: &ctx_provider_runtime::provider_guard::ProviderGuardEvent,
) {
    let session_ids = state.sessions.list_running_sessions().await;
    for session_id in session_ids {
        let store = match state.store_for_session(session_id).await {
            Ok(store) => store,
            Err(_) => continue,
        };
        let session = store.get_session(session_id).await.ok().flatten();
        let Some(session) = session else {
            continue;
        };
        if session.provider_id != event.sample.label {
            continue;
        }
        let payload = json!({
            "provider": event.sample.label,
            "kind": event.kind,
            "stage": event.stage,
            "pid": event.sample.pid,
            "memory_mb": bytes_to_mb(event.sample.memory_bytes),
            "system_total_mb": bytes_to_mb(event.system.memory_total_bytes),
            "system_used_mb": bytes_to_mb(event.system.memory_used_bytes),
            "limit_high_mb": event.limits.memory_high_mb,
            "limit_max_mb": event.limits.memory_max_mb,
            "grace_period_ms": event.limits.grace_period.as_millis() as u64,
            "kill_at_ms": event.kill_at_ms,
            "message": match event.kind {
                "provider_guard_warning" => "Provider memory is above the guard threshold.",
                "provider_guard_kill" => "Provider process killed after exceeding memory limits.",
                _ => "Provider guard notice.",
            },
        });
        match store
            .append_session_event(session_id, None, None, SessionEventType::Notice, payload)
            .await
        {
            Ok(event) => state.publish_event(event).await,
            Err(err) => tracing::warn!(
                provider_id = %session.provider_id,
                session_id = %session_id.0,
                "provider guard failed to append session event: {err:#}"
            ),
        }
    }
}

async fn list_provider_processes(
    state: &AppState,
) -> Vec<ctx_providers::adapters::ProviderProcessInfo> {
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

fn bytes_to_mb(value: u64) -> u64 {
    value / (1024 * 1024)
}

#[cfg(target_os = "linux")]
fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
