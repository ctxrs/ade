use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use chrono::Utc;
use serde_json::json;
use sysinfo::{Pid, Signal, System};

use ctx_core::ids::MessageId;
use ctx_core::models::{Message, MessageDelivery, MessageRole, SessionEventType};
use ctx_providers::adapters::ProviderRestartMode;

use crate::daemon::AppState;
use crate::resource_utilization::{ProviderMemorySample, SystemSnapshot};
use crate::settings::{ProviderRestartSettings, ResourceGovernanceMode, Settings};

const DEFAULT_INTERVAL_MS: u64 = 5_000;
const DEFAULT_GRACE_PERIOD_MS: u64 = 300_000;
const DEFAULT_MIN_MEMORY_MB: u64 = 1024;
const DEFAULT_MEMORY_FRACTION: f64 = 0.6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRestartLimits {
    pub memory_high_mb: u32,
    pub memory_max_mb: u32,
    pub interval: Duration,
    pub grace_period: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderRestartRuntime {
    pub enabled: bool,
    pub last_applied: Option<ProviderRestartLimits>,
    pub last_message: Option<String>,
}

#[derive(Debug, Clone)]
struct OverLimitState {
    first_seen: Instant,
    last_seen: Instant,
    last_memory_bytes: u64,
    last_tool_memory_bytes: u64,
    pid: u32,
}

pub fn compute_effective_limits(
    settings: &ProviderRestartSettings,
    system: &SystemSnapshot,
) -> Option<ProviderRestartLimits> {
    if !settings.enabled {
        return None;
    }

    let total_mb = (system.memory_total_bytes / (1024 * 1024)).max(1);
    let mut memory_max_mb = match settings.mode {
        ResourceGovernanceMode::Auto => {
            ((total_mb as f64) * DEFAULT_MEMORY_FRACTION).round() as u64
        }
        ResourceGovernanceMode::Custom => settings.memory_max_mb.unwrap_or(0) as u64,
    };
    if memory_max_mb == 0 {
        memory_max_mb = ((total_mb as f64) * DEFAULT_MEMORY_FRACTION).round() as u64;
    }
    memory_max_mb = memory_max_mb.max(DEFAULT_MIN_MEMORY_MB);

    let mut memory_high_mb = match settings.mode {
        ResourceGovernanceMode::Auto => ((memory_max_mb as f64) * 0.9).round() as u64,
        ResourceGovernanceMode::Custom => settings.memory_high_mb.unwrap_or(0) as u64,
    };
    if memory_high_mb == 0 {
        memory_high_mb = ((memory_max_mb as f64) * 0.9).round() as u64;
    }
    if memory_high_mb > memory_max_mb {
        memory_high_mb = memory_max_mb;
    }

    let interval_ms = settings.interval_ms.unwrap_or(DEFAULT_INTERVAL_MS).max(100);
    let grace_ms = settings.grace_period_ms.unwrap_or(DEFAULT_GRACE_PERIOD_MS);

    Some(ProviderRestartLimits {
        memory_high_mb: memory_high_mb as u32,
        memory_max_mb: memory_max_mb as u32,
        interval: Duration::from_millis(interval_ms),
        grace_period: Duration::from_millis(grace_ms),
    })
}

pub async fn apply_settings(state: &AppState, settings: &Settings) -> Result<()> {
    let cfg = settings.provider_restart.clone().unwrap_or_default();
    let (system, _disks, _cache_age_ms) = {
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        sampler.system_snapshot()
    };
    let effective = compute_effective_limits(&cfg, &system);

    let runtime = ProviderRestartRuntime {
        enabled: cfg.enabled,
        last_applied: effective,
        last_message: None,
    };

    let mut guard = state.providers.restart.lock().await;
    *guard = runtime;
    Ok(())
}

pub fn spawn_provider_restart(state: Arc<AppState>) {
    let mut shutdown_rx = state.core.shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut over_high: HashMap<String, OverLimitState> = HashMap::new();

        loop {
            let (enabled, limits) = {
                let runtime = state.providers.restart.lock().await;
                (runtime.enabled, runtime.last_applied.clone())
            };

            if !enabled || limits.is_none() {
                over_high.clear();
                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                }
                continue;
            }

            if let Some(limits) = limits.as_ref() {
                if let Err(err) = restart_once(&state, limits, &mut over_high).await {
                    tracing::warn!("provider restart tick failed: {err:#}");
                }
            }

            let interval = limits
                .as_ref()
                .map(|l| l.interval)
                .unwrap_or(Duration::from_millis(DEFAULT_INTERVAL_MS));
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                _ = tokio::time::sleep(interval) => {},
            }
        }
    });
}

async fn restart_once(
    state: &Arc<AppState>,
    limits: &ProviderRestartLimits,
    over_high: &mut HashMap<String, OverLimitState>,
) -> Result<()> {
    let provider_processes = list_provider_processes(state).await;
    let (system, samples) = {
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        let (system, _disks, _cache_age_ms) = sampler.system_snapshot();
        let samples = sampler.provider_memory_snapshot(&provider_processes);
        (system, samples)
    };

    handle_limits(state, limits, &system, &samples, over_high).await?;
    Ok(())
}

async fn handle_limits(
    state: &Arc<AppState>,
    limits: &ProviderRestartLimits,
    system: &SystemSnapshot,
    samples: &[ProviderMemorySample],
    over_high: &mut HashMap<String, OverLimitState>,
) -> Result<()> {
    let mut seen: HashSet<String> = HashSet::new();
    let high_bytes = mb_to_bytes(limits.memory_high_mb);

    for sample in samples {
        let provider_id = sample.provider_id.clone();
        seen.insert(provider_id.clone());

        if sample.memory_bytes < high_bytes {
            over_high.remove(&provider_id);
            continue;
        }

        let now = Instant::now();
        let entry = over_high.entry(provider_id.clone());
        let mut is_new = false;
        match entry {
            std::collections::hash_map::Entry::Vacant(vacant) => {
                is_new = true;
                vacant.insert(OverLimitState {
                    first_seen: now,
                    last_seen: now,
                    last_memory_bytes: sample.memory_bytes,
                    last_tool_memory_bytes: sample.tool_memory_bytes,
                    pid: sample.pid,
                });
            }
            std::collections::hash_map::Entry::Occupied(mut occupied) => {
                let entry = occupied.get_mut();
                if entry.pid != sample.pid {
                    is_new = true;
                    *entry = OverLimitState {
                        first_seen: now,
                        last_seen: now,
                        last_memory_bytes: sample.memory_bytes,
                        last_tool_memory_bytes: sample.tool_memory_bytes,
                        pid: sample.pid,
                    };
                } else {
                    entry.last_seen = now;
                    entry.last_memory_bytes = sample.memory_bytes;
                    entry.last_tool_memory_bytes = sample.tool_memory_bytes;
                }
            }
        }

        if is_new {
            let restart_at_ms =
                unix_ms_now().saturating_add(limits.grace_period.as_millis() as u64);
            notify_sessions(
                state,
                sample,
                limits,
                system,
                "provider_restart_warning",
                "high",
                Some(restart_at_ms),
            )
            .await;
        }

        let entry = over_high
            .get(&provider_id)
            .expect("entry exists after update");
        if now.duration_since(entry.first_seen) >= limits.grace_period {
            notify_sessions(
                state,
                sample,
                limits,
                system,
                "provider_restart",
                "restart",
                None,
            )
            .await;
            restart_provider(state, &provider_id, sample.pid).await;
            over_high.remove(&provider_id);
        }
    }

    over_high.retain(|provider_id, _| seen.contains(provider_id));
    Ok(())
}

async fn restart_provider(state: &Arc<AppState>, provider_id: &str, pid: u32) {
    let adapter = {
        let providers = state.providers.adapters.lock().await;
        providers.get(provider_id).cloned()
    };
    let mut needs_kill = true;
    if let Some(adapter) = adapter {
        match adapter
            .restart(
                "provider restart: sustained memory usage",
                ProviderRestartMode::Immediate,
            )
            .await
        {
            Ok(()) => {
                needs_kill = false;
            }
            Err(err) => {
                tracing::warn!(
                    provider_id = provider_id,
                    pid = pid,
                    "provider restart hook failed: {err:#}"
                );
            }
        }
    } else {
        tracing::warn!(
            provider_id = provider_id,
            pid = pid,
            "provider restart failed: provider adapter missing"
        );
    }

    if needs_kill {
        let killed = signal_pids(&[pid], Signal::Kill);
        if killed == 0 {
            tracing::warn!(
                provider_id = provider_id,
                pid = pid,
                "provider restart failed to kill process"
            );
        }
    }
}

async fn notify_sessions(
    state: &Arc<AppState>,
    sample: &ProviderMemorySample,
    limits: &ProviderRestartLimits,
    system: &SystemSnapshot,
    kind: &str,
    stage: &str,
    restart_at_ms: Option<u64>,
) {
    let message_text = match kind {
        "provider_restart_warning" => {
            "Provider memory is high; restart scheduled if it stays elevated."
        }
        "provider_restart" => "Provider restart requested after sustained high memory usage.",
        _ => "Provider restart notice.",
    };
    let session_ids = state.list_running_sessions().await;
    for session_id in session_ids {
        let store = match state.store_for_session(session_id).await {
            Ok(store) => store,
            Err(_) => continue,
        };
        let session = store.get_session(session_id).await.ok().flatten();
        let Some(session) = session else {
            continue;
        };
        if session.provider_id != sample.provider_id {
            continue;
        }

        let message_id = insert_system_message(state, &store, &session, message_text).await;
        let payload = json!({
            "provider": sample.provider_id,
            "kind": kind,
            "stage": stage,
            "pid": sample.pid,
            "memory_mb": bytes_to_mb(sample.memory_bytes),
            "tool_memory_mb": bytes_to_mb(sample.tool_memory_bytes),
            "system_total_mb": bytes_to_mb(system.memory_total_bytes),
            "system_used_mb": bytes_to_mb(system.memory_used_bytes),
            "limit_high_mb": limits.memory_high_mb,
            "limit_max_mb": limits.memory_max_mb,
            "grace_period_ms": limits.grace_period.as_millis() as u64,
            "restart_at_ms": restart_at_ms,
            "message": message_text,
            "message_id": message_id.map(|id| id.0),
        });
        match store
            .append_session_event(session_id, None, None, SessionEventType::Notice, payload)
            .await
        {
            Ok(event) => state.publish_event(event).await,
            Err(err) => tracing::warn!(
                provider_id = %sample.provider_id,
                session_id = %session_id.0,
                "provider restart failed to append session event: {err:#}"
            ),
        }
    }
}

async fn insert_system_message(
    state: &crate::daemon::AppState,
    store: &ctx_store::Store,
    session: &ctx_core::models::Session,
    content: &str,
) -> Option<MessageId> {
    let now = Utc::now();
    let message_id = MessageId::new();
    let order_seq_state = state
        .sessions
        .get_order_seq_state(store, session.id)
        .await;
    let order_seq = {
        let mut order_seq_state = order_seq_state.lock().await;
        order_seq_state.get_or_assign(format!("message:{}", message_id.0), None)
    };
    let msg = Message {
        id: message_id,
        session_id: session.id,
        task_id: session.task_id,
        run_id: None,
        turn_id: None,
        turn_sequence: None,
        order_seq: Some(order_seq),
        role: MessageRole::System,
        content: content.to_string(),
        attachments: vec![],
        delivery: MessageDelivery::Immediate,
        delivered_at: Some(now),
        created_at: now,
    };
    match store.insert_message(msg).await {
        Ok(saved) => Some(saved.id),
        Err(err) => {
            tracing::warn!(
                session_id = %session.id.0,
                "provider restart failed to insert system message: {err:#}"
            );
            None
        }
    }
}

fn signal_pids(pids: &[u32], signal: Signal) -> usize {
    let mut system = System::new();
    system.refresh_processes();
    let mut killed = 0usize;
    for pid in pids {
        if let Some(process) = system.process(Pid::from_u32(*pid)) {
            if process.kill_with(signal).unwrap_or(false) {
                killed += 1;
            }
        }
    }
    killed
}

async fn list_provider_processes(
    state: &Arc<AppState>,
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

fn mb_to_bytes(value: u32) -> u64 {
    value as u64 * 1024 * 1024
}

fn bytes_to_mb(value: u64) -> u64 {
    value / (1024 * 1024)
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
