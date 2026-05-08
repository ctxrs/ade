use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use ctx_core::ids::SessionId;
pub(crate) use ctx_storage_admission::{
    is_storage_exhaustion_error, storage_emergency_message, storage_exhaustion_message,
    StorageGuardLevel, StorageGuardObservedPath, StorageGuardReserveAction,
    StorageGuardReserveWarning, StorageGuardRuntime, StorageGuardStatus,
    STORAGE_GUARD_MONITOR_INTERVAL,
};
#[cfg(test)]
pub(crate) use ctx_storage_admission::{StorageGuardPathStatus, STORAGE_GUARD_RESERVE_FILE_NAME};

use crate::daemon::scheduler::SchedulerCommand;
use crate::daemon::AppState;
use ctx_observability::ops_events::OpsEvent;

#[cfg(test)]
const GIB: u64 = ctx_storage_admission::STORAGE_BYTES_GIB;
#[cfg(test)]
const MIB: u64 = ctx_storage_admission::STORAGE_BYTES_MIB;

pub fn spawn_storage_guard(state: Arc<AppState>) {
    let mut shutdown_rx = state.core.shutdown_tx.subscribe();
    tokio::spawn(async move {
        if let Err(err) = evaluate_storage_guard(&state, &[]).await {
            tracing::warn!("storage guard initial evaluation failed: {err:#}");
        }

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                _ = tokio::time::sleep(STORAGE_GUARD_MONITOR_INTERVAL) => {
                    if let Err(err) = evaluate_storage_guard(&state, &[]).await {
                        tracing::warn!("storage guard tick failed: {err:#}");
                    }
                }
            }
        }
    });
}

pub async fn preflight_turn_start(state: &Arc<AppState>, workdir: &Path) -> Result<()> {
    let current = state.storage_guard_snapshot();
    if current.is_emergency() {
        anyhow::bail!(storage_emergency_message(current.active.as_ref()));
    }
    let snapshot = refresh_preflight_storage_guard(state, &[workdir.to_path_buf()]).await;
    if snapshot.is_emergency() {
        anyhow::bail!(storage_emergency_message(snapshot.active.as_ref()));
    }
    Ok(())
}

pub async fn evaluate_storage_guard(
    state: &Arc<AppState>,
    extra_paths: &[PathBuf],
) -> Result<StorageGuardStatus> {
    let observed_paths = collect_observed_paths(state, extra_paths).await;
    let disks = sample_storage_disks(state).await;
    let (previous, snapshot, warnings) = state
        .core
        .storage_guard
        .evaluate(&state.core.data_root, &observed_paths, &disks)
        .await;
    emit_reserve_warnings(warnings);

    publish_storage_guard_snapshot(state, &previous, &snapshot).await;
    Ok(snapshot)
}

async fn refresh_preflight_storage_guard(
    state: &Arc<AppState>,
    extra_paths: &[PathBuf],
) -> StorageGuardStatus {
    let observed_paths = collect_observed_paths(state, extra_paths).await;
    let disks = sample_storage_disks(state).await;
    let (previous, snapshot) =
        state
            .core
            .storage_guard
            .sample_preflight(&state.core.data_root, &observed_paths, &disks);
    publish_storage_guard_snapshot(state, &previous, &snapshot).await;
    snapshot
}

async fn publish_storage_guard_snapshot(
    state: &Arc<AppState>,
    previous: &StorageGuardStatus,
    snapshot: &StorageGuardStatus,
) {
    let should_interrupt = previous.level != StorageGuardLevel::Emergency
        && snapshot.level == StorageGuardLevel::Emergency;
    if !snapshot.same_meaningful_state(previous) {
        emit_storage_guard_transition(state, snapshot);
    }
    state.core.storage_guard.publish(snapshot.clone());
    if should_interrupt {
        dispatch_storage_emergency_interrupts(state, snapshot).await;
    }
}

fn emit_storage_guard_transition(state: &AppState, snapshot: &StorageGuardStatus) {
    let mut event = OpsEvent::new(
        match snapshot.level {
            StorageGuardLevel::Emergency => "error",
            StorageGuardLevel::Warning => "warning",
            StorageGuardLevel::Normal => "info",
        },
        "storage_guard_state_changed",
    );
    event.meta = Some(json!({
        "level": snapshot.level,
        "reserve_file_active": snapshot.reserve_file_active,
        "active": snapshot.active,
    }));
    state.telemetry.ops_events.emit(event);
}

fn emit_reserve_warnings(warnings: Vec<StorageGuardReserveWarning>) {
    for warning in warnings {
        match warning.action {
            StorageGuardReserveAction::Allocate => {
                tracing::warn!(
                    reserve_file = %warning.reserve_file_path.to_string_lossy(),
                    "failed to allocate storage reserve file: {:#}",
                    warning.message
                );
            }
            StorageGuardReserveAction::Release => {
                tracing::warn!(
                    reserve_file = %warning.reserve_file_path.to_string_lossy(),
                    "failed to release storage reserve file: {:#}",
                    warning.message
                );
            }
        }
    }
}

async fn dispatch_storage_emergency_interrupts(
    state: &Arc<AppState>,
    snapshot: &StorageGuardStatus,
) {
    let running_sessions = state.sessions.list_running_sessions().await;
    let mut interrupted = 0usize;
    for session_id in running_sessions {
        if dispatch_storage_emergency_interrupt(state, session_id).await {
            interrupted += 1;
        }
    }

    tracing::warn!(
        interrupted_sessions = interrupted,
        level = ?snapshot.level,
        active_path = snapshot.active.as_ref().map(|path| path.path.as_str()),
        "storage emergency interrupted active sessions"
    );
}

async fn dispatch_storage_emergency_interrupt(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> bool {
    let Some(tx) = state.sessions.scheduler_sender(session_id).await else {
        return false;
    };
    tx.send(SchedulerCommand::StorageEmergency).await.is_ok()
}

async fn sample_storage_disks(
    state: &Arc<AppState>,
) -> Vec<ctx_resource_utilization::DiskSnapshot> {
    let mut sampler = state.telemetry.resource_sampler.lock().await;
    let (_system, disks, _cache_age_ms) = sampler.system_snapshot();
    disks
}

async fn collect_observed_paths(
    state: &Arc<AppState>,
    extra_paths: &[PathBuf],
) -> Vec<StorageGuardObservedPath> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    push_observed_path(
        &mut paths,
        &mut seen,
        "CTX data root",
        state.core.data_root.clone(),
    );
    push_observed_path(&mut paths, &mut seen, "temp storage", std::env::temp_dir());

    for workdir in running_session_workdirs(state).await {
        push_observed_path(&mut paths, &mut seen, "active worktree", workdir);
    }
    for workdir in extra_paths {
        push_observed_path(&mut paths, &mut seen, "active worktree", workdir.clone());
    }
    paths
}

fn push_observed_path(
    paths: &mut Vec<StorageGuardObservedPath>,
    seen: &mut HashSet<PathBuf>,
    label: &'static str,
    path: PathBuf,
) {
    if !seen.insert(path.clone()) {
        return;
    }
    paths.push(StorageGuardObservedPath::new(label, path));
}

async fn running_session_workdirs(state: &Arc<AppState>) -> Vec<PathBuf> {
    let mut workdirs = Vec::new();
    for session_id in state.sessions.list_running_sessions().await {
        let Ok(store) = state.store_for_session(session_id).await else {
            continue;
        };
        let Ok(Some(session)) = store.get_session(session_id).await else {
            continue;
        };
        let Ok(Some(worktree)) = store.get_worktree(session.worktree_id).await else {
            continue;
        };
        workdirs.push(PathBuf::from(worktree.root_path));
    }
    workdirs
}

impl AppState {
    pub fn storage_guard_snapshot(&self) -> StorageGuardStatus {
        self.core.storage_guard.snapshot()
    }
}

#[cfg(test)]
mod tests;
