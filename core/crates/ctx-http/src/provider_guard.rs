use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde_json::json;
use sysinfo::{Pid, Signal, System};

use ctx_core::models::SessionEventType;

use crate::daemon::AppState;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::resource_utilization::{ResourceProcess, ResourceProcesses, SystemSnapshot};
use crate::settings::{ProviderGuardSettings, ResourceGovernanceMode, Settings};

const DEFAULT_INTERVAL_MS: u64 = 5_000;
const DEFAULT_GRACE_PERIOD_MS: u64 = 15_000;
const DEFAULT_MIN_MEMORY_MB: u64 = 1024;
const DEFAULT_MEMORY_FRACTION: f64 = 0.25;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderGuardLimits {
    pub memory_high_mb: u32,
    pub memory_max_mb: u32,
    pub interval: Duration,
    pub grace_period: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderGuardRuntime {
    pub enabled: bool,
    pub last_applied: Option<ProviderGuardLimits>,
    pub last_message: Option<String>,
}

#[derive(Debug, Clone)]
struct OverLimitState {
    first_seen: Instant,
    last_seen: Instant,
    last_memory_bytes: u64,
}

pub fn compute_effective_limits(
    settings: &ProviderGuardSettings,
    system: &SystemSnapshot,
) -> Option<ProviderGuardLimits> {
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

    Some(ProviderGuardLimits {
        memory_high_mb: memory_high_mb as u32,
        memory_max_mb: memory_max_mb as u32,
        interval: Duration::from_millis(interval_ms),
        grace_period: Duration::from_millis(grace_ms),
    })
}

pub async fn apply_settings(state: &AppState, settings: &Settings) -> Result<()> {
    let cfg = settings.provider_guard.clone().unwrap_or_default();
    let (system, _disks, _cache_age_ms) = {
        let mut sampler = state.resource_sampler.lock().await;
        sampler.system_snapshot()
    };
    let effective = compute_effective_limits(&cfg, &system);

    let runtime = ProviderGuardRuntime {
        enabled: cfg.enabled,
        last_applied: effective,
        last_message: None,
    };

    let mut guard = state.provider_guard.lock().await;
    *guard = runtime;
    Ok(())
}

pub fn spawn_provider_guard(state: Arc<AppState>) {
    let mut shutdown_rx = state.shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut warned_high: HashSet<u32> = HashSet::new();
        let mut over_max: HashMap<u32, OverLimitState> = HashMap::new();

        loop {
            let (enabled, limits) = {
                let runtime = state.provider_guard.lock().await;
                (runtime.enabled, runtime.last_applied.clone())
            };

            if !enabled || limits.is_none() {
                warned_high.clear();
                over_max.clear();
                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                }
                continue;
            }

            if let Some(limits) = limits.as_ref() {
                if let Err(err) = guard_once(&state, limits, &mut warned_high, &mut over_max).await
                {
                    tracing::warn!("provider guard tick failed: {err:#}");
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

async fn guard_once(
    state: &Arc<AppState>,
    limits: &ProviderGuardLimits,
    warned_high: &mut HashSet<u32>,
    over_max: &mut HashMap<u32, OverLimitState>,
) -> Result<()> {
    let provider_processes = list_provider_processes(state).await;
    let (system, processes) = {
        let mut sampler = state.resource_sampler.lock().await;
        let (system, _disks, _cache_age_ms) = sampler.system_snapshot();
        let processes = sampler.processes_snapshot(std::process::id(), &provider_processes);
        (system, processes)
    };

    handle_limits(state, limits, &system, &processes, warned_high, over_max).await?;
    Ok(())
}

async fn handle_limits(
    state: &Arc<AppState>,
    limits: &ProviderGuardLimits,
    system: &SystemSnapshot,
    processes: &ResourceProcesses,
    warned_high: &mut HashSet<u32>,
    over_max: &mut HashMap<u32, OverLimitState>,
) -> Result<()> {
    let mut seen: HashSet<u32> = HashSet::new();
    let high_bytes = mb_to_bytes(limits.memory_high_mb);
    let max_bytes = mb_to_bytes(limits.memory_max_mb);

    for proc in processes.providers.iter() {
        let pid = proc.pid;
        seen.insert(pid);

        if proc.memory_bytes >= high_bytes && !warned_high.contains(&pid) {
            warned_high.insert(pid);
            log_guard_event(state, proc, "over_high", limits, proc.memory_bytes).await;
            notify_sessions(
                state,
                proc,
                limits,
                system,
                "provider_guard_warning",
                "high",
                None,
            )
            .await;
        } else if proc.memory_bytes < high_bytes {
            warned_high.remove(&pid);
        }

        if proc.memory_bytes >= max_bytes {
            let now = Instant::now();
            let is_new = !over_max.contains_key(&pid);
            let entry = over_max.entry(pid).or_insert_with(|| OverLimitState {
                first_seen: now,
                last_seen: now,
                last_memory_bytes: proc.memory_bytes,
            });
            entry.last_seen = now;
            entry.last_memory_bytes = proc.memory_bytes;

            if is_new {
                log_guard_event(state, proc, "over_max", limits, proc.memory_bytes).await;
                capture_guard_snapshot(state, proc, "over_max", proc.memory_bytes).await;
                let kill_at_ms =
                    unix_ms_now().saturating_add(limits.grace_period.as_millis() as u64);
                notify_sessions(
                    state,
                    proc,
                    limits,
                    system,
                    "provider_guard_warning",
                    "max",
                    Some(kill_at_ms),
                )
                .await;
            }

            if now.duration_since(entry.first_seen) >= limits.grace_period {
                kill_provider_process(state, proc, limits, system).await;
                over_max.remove(&pid);
            }
        } else {
            over_max.remove(&pid);
        }
    }

    warned_high.retain(|pid| seen.contains(pid));
    over_max.retain(|pid, _| seen.contains(pid));
    Ok(())
}

async fn log_guard_event(
    state: &Arc<AppState>,
    proc: &ResourceProcess,
    event: &str,
    limits: &ProviderGuardLimits,
    memory_bytes: u64,
) {
    let mem_mb = bytes_to_mb(memory_bytes);
    tracing::warn!(
        provider_id = %proc.label,
        pid = proc.pid,
        event,
        memory_mb = mem_mb,
        limit_high_mb = limits.memory_high_mb,
        limit_max_mb = limits.memory_max_mb,
        "provider guard triggered"
    );

    let mut labels = HashMap::new();
    labels.insert("provider_id".to_string(), proc.label.clone());
    labels.insert("event".to_string(), event.to_string());
    state
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

async fn kill_provider_process(
    state: &Arc<AppState>,
    proc: &ResourceProcess,
    limits: &ProviderGuardLimits,
    system: &SystemSnapshot,
) {
    log_guard_event(state, proc, "kill", limits, proc.memory_bytes).await;
    capture_guard_snapshot(state, proc, "kill", proc.memory_bytes).await;
    notify_sessions(
        state,
        proc,
        limits,
        system,
        "provider_guard_kill",
        "kill",
        None,
    )
    .await;

    let mut pids = Vec::new();
    pids.push(proc.pid);
    for child in proc.children.iter() {
        pids.push(child.pid);
    }
    let killed = signal_pids(&pids, Signal::Kill);
    if killed == 0 {
        tracing::warn!(
            provider_id = %proc.label,
            pid = proc.pid,
            "provider guard failed to kill process"
        );
    }
}

async fn capture_guard_snapshot(
    state: &Arc<AppState>,
    proc: &ResourceProcess,
    event: &str,
    memory_bytes: u64,
) {
    #[cfg(target_os = "linux")]
    {
        let timestamp_ms = unix_ms_now();
        let dir = state.data_root.join("logs").join("providers");
        let path = dir.join(format!(
            "provider-guard-{}-{}-{}-{}.log",
            proc.label, proc.pid, event, timestamp_ms
        ));
        if tokio::fs::create_dir_all(&dir).await.is_err() {
            return;
        }

        let mut output = String::new();
        output.push_str(&format!("event={event}\n"));
        output.push_str(&format!("pid={}\n", proc.pid));
        output.push_str(&format!("label={}\n", proc.label));
        output.push_str(&format!("memory_bytes={memory_bytes}\n"));
        output.push_str(&format!("timestamp_ms={timestamp_ms}\n\n"));

        let status_path = format!("/proc/{}/status", proc.pid);
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

        let smaps_path = format!("/proc/{}/smaps_rollup", proc.pid);
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

        let cmdline_path = format!("/proc/{}/cmdline", proc.pid);
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
                provider_id = %proc.label,
                pid = proc.pid,
                path = %path.display(),
                "captured provider guard snapshot"
            );
        }
    }
}

async fn notify_sessions(
    state: &Arc<AppState>,
    proc: &ResourceProcess,
    limits: &ProviderGuardLimits,
    system: &SystemSnapshot,
    kind: &str,
    stage: &str,
    kill_at_ms: Option<u64>,
) {
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
        if session.provider_id != proc.label {
            continue;
        }
        let payload = json!({
            "provider": proc.label,
            "kind": kind,
            "stage": stage,
            "pid": proc.pid,
            "memory_mb": bytes_to_mb(proc.memory_bytes),
            "system_total_mb": bytes_to_mb(system.memory_total_bytes),
            "system_used_mb": bytes_to_mb(system.memory_used_bytes),
            "limit_high_mb": limits.memory_high_mb,
            "limit_max_mb": limits.memory_max_mb,
            "grace_period_ms": limits.grace_period.as_millis() as u64,
            "kill_at_ms": kill_at_ms,
            "message": match kind {
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
                provider_id = %proc.label,
                session_id = %session_id.0,
                "provider guard failed to append session event: {err:#}"
            ),
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
        let providers = state.providers.lock().await;
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
