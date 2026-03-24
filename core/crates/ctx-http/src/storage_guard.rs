use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use fs2::FileExt;
use serde::Serialize;
use serde_json::json;
use tokio::sync::Mutex;

use ctx_core::ids::SessionId;

use crate::daemon::AppState;
use crate::ops_events::OpsEvent;
use crate::resource_utilization::{disk_for_path, DiskSnapshot};
use crate::scheduler::SchedulerCommand;

const GIB: u64 = 1024 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;
const WARNING_FREE_BYTES: u64 = 2 * GIB;
const EMERGENCY_FREE_BYTES: u64 = GIB;
const RESERVE_BYTES: u64 = 512 * MIB;
const MONITOR_INTERVAL: Duration = Duration::from_secs(2);
const RESERVE_FILE_NAME: &str = ".storage-guard.reserve";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageGuardLevel {
    #[default]
    Normal,
    Warning,
    Emergency,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StorageGuardPathStatus {
    pub label: String,
    pub path: String,
    pub mount_point: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StorageGuardStatus {
    pub level: StorageGuardLevel,
    pub warning_threshold_bytes: u64,
    pub emergency_threshold_bytes: u64,
    pub reserve_bytes: u64,
    pub reserve_file_active: bool,
    pub active: Option<StorageGuardPathStatus>,
    pub updated_at: String,
}

impl Default for StorageGuardStatus {
    fn default() -> Self {
        Self {
            level: StorageGuardLevel::Normal,
            warning_threshold_bytes: WARNING_FREE_BYTES,
            emergency_threshold_bytes: EMERGENCY_FREE_BYTES,
            reserve_bytes: RESERVE_BYTES,
            reserve_file_active: false,
            active: None,
            updated_at: Utc::now().to_rfc3339(),
        }
    }
}

impl StorageGuardStatus {
    pub fn is_emergency(&self) -> bool {
        self.level == StorageGuardLevel::Emergency
    }
}

#[derive(Default)]
struct StorageGuardController {
    reserve_file_active: bool,
}

pub struct StorageGuardRuntime {
    controller: Mutex<StorageGuardController>,
    reserve_file_path: PathBuf,
    snapshot: RwLock<StorageGuardStatus>,
}

impl StorageGuardRuntime {
    pub fn new(data_root: &Path) -> Self {
        Self {
            controller: Mutex::new(StorageGuardController::default()),
            reserve_file_path: data_root.join(RESERVE_FILE_NAME),
            snapshot: RwLock::new(StorageGuardStatus::default()),
        }
    }

    pub fn snapshot(&self) -> StorageGuardStatus {
        self.snapshot
            .read()
            .expect("storage guard snapshot lock poisoned") // EXCEPTION: panic-trap — critical section is a trivial clone; poisoning means a prior panic already occurred
            .clone()
    }

    pub fn publish(&self, snapshot: StorageGuardStatus) {
        *self
            .snapshot
            .write()
            .expect("storage guard snapshot lock poisoned") = snapshot; // EXCEPTION: panic-trap — critical section is a trivial assignment; poisoning means a prior panic already occurred
    }
}

#[derive(Clone)]
struct ObservedPath {
    label: &'static str,
    path: PathBuf,
}

struct StorageAssessment {
    status: StorageGuardStatus,
    reserve_mount_point: Option<String>,
}

pub fn spawn_storage_guard(state: Arc<AppState>) {
    let mut shutdown_rx = state.core.shutdown_tx.subscribe();
    tokio::spawn(async move {
        if let Err(err) = evaluate_storage_guard(&state, &[]).await {
            tracing::warn!("storage guard initial evaluation failed: {err:#}");
        }

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                _ = tokio::time::sleep(MONITOR_INTERVAL) => {
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
    let snapshot = evaluate_storage_guard(state, &[workdir.to_path_buf()]).await?;
    if snapshot.is_emergency() {
        anyhow::bail!(storage_emergency_message(snapshot.active.as_ref()));
    }
    Ok(())
}

pub fn storage_exhaustion_message(active: Option<&StorageGuardPathStatus>) -> String {
    match active {
        Some(path) => format!(
            "Storage exhausted while saving assistant output on {}. Free space, then retry the session.",
            format_path_label(path)
        ),
        None => {
            "Storage exhausted while saving assistant output. Free space, then retry the session."
                .to_string()
        }
    }
}

pub fn storage_emergency_message(active: Option<&StorageGuardPathStatus>) -> String {
    match active {
        Some(path) => format!(
            "Storage is critically low on {}. CTX stopped agent runs to protect local data. Free space, then retry the session.",
            format_path_label(path)
        ),
        None => {
            "Storage is critically low. CTX stopped agent runs to protect local data. Free space, then retry the session."
                .to_string()
        }
    }
}

pub fn is_storage_exhaustion_error(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("database or disk is full")
        || normalized.contains("no space left on device")
        || normalized.contains("sqlite_full")
        || normalized.contains("os error 28")
}

pub async fn evaluate_storage_guard(
    state: &Arc<AppState>,
    extra_paths: &[PathBuf],
) -> Result<StorageGuardStatus> {
    let (snapshot, should_interrupt) = {
        let mut controller = state.core.storage_guard.controller.lock().await;
        let previous = state.core.storage_guard.snapshot();

        let mut assessment =
            sample_storage_assessment(state, extra_paths, controller.reserve_file_active).await;

        if assessment.status.level == StorageGuardLevel::Normal && !controller.reserve_file_active {
            if let Err(err) = ensure_reserve_file(&state.core.storage_guard.reserve_file_path) {
                tracing::warn!(
                    reserve_file = %state.core.storage_guard.reserve_file_path.to_string_lossy(),
                    "failed to allocate storage reserve file: {err:#}"
                );
            } else {
                controller.reserve_file_active = true;
                assessment =
                    sample_storage_assessment(state, extra_paths, controller.reserve_file_active)
                        .await;
            }
        }

        let should_release_reserve = assessment.status.level == StorageGuardLevel::Emergency
            && controller.reserve_file_active
            && assessment
                .status
                .active
                .as_ref()
                .and_then(|active| {
                    assessment
                        .reserve_mount_point
                        .as_ref()
                        .map(|reserve_mount| active.mount_point == *reserve_mount)
                })
                .unwrap_or(false);

        if should_release_reserve {
            if let Err(err) = release_reserve_file(&state.core.storage_guard.reserve_file_path) {
                tracing::warn!(
                    reserve_file = %state.core.storage_guard.reserve_file_path.to_string_lossy(),
                    "failed to release storage reserve file: {err:#}"
                );
            } else {
                controller.reserve_file_active = false;
                assessment =
                    sample_storage_assessment(state, extra_paths, controller.reserve_file_active)
                        .await;
            }
        }

        let snapshot = assessment.status;
        let should_interrupt = previous.level != StorageGuardLevel::Emergency
            && snapshot.level == StorageGuardLevel::Emergency;
        if snapshot != previous {
            emit_storage_guard_transition(state, &snapshot);
            state.core.storage_guard.publish(snapshot.clone());
        }

        (snapshot, should_interrupt)
    };

    if should_interrupt {
        dispatch_storage_emergency_interrupts(state, &snapshot).await;
    }

    Ok(snapshot)
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

async fn dispatch_storage_emergency_interrupts(
    state: &Arc<AppState>,
    snapshot: &StorageGuardStatus,
) {
    let running_sessions = state.list_running_sessions().await;
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
    let Some(tx) = state.scheduler_sender(session_id).await else {
        return false;
    };
    tx.send(SchedulerCommand::StorageEmergency).await.is_ok()
}

async fn sample_storage_assessment(
    state: &Arc<AppState>,
    extra_paths: &[PathBuf],
    reserve_file_active: bool,
) -> StorageAssessment {
    let observed_paths = collect_observed_paths(state, extra_paths).await;
    let disks = {
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        let (_system, disks, _cache_age_ms) = sampler.system_snapshot();
        disks
    };
    build_storage_assessment(
        &state.core.data_root,
        &observed_paths,
        &disks,
        reserve_file_active,
    )
}

fn build_storage_assessment(
    data_root: &Path,
    observed_paths: &[ObservedPath],
    disks: &[DiskSnapshot],
    reserve_file_active: bool,
) -> StorageAssessment {
    let reserve_mount_point = disk_for_path(data_root, disks).map(|disk| disk.mount_point);
    let mut active: Option<StorageGuardPathStatus> = None;
    for observed in observed_paths {
        let Some(disk) = disk_for_path(&observed.path, disks) else {
            continue;
        };
        let reserve_bonus = if reserve_file_active
            && reserve_mount_point
                .as_deref()
                .map(|mount| mount == disk.mount_point)
                .unwrap_or(false)
        {
            RESERVE_BYTES
        } else {
            0
        };
        let sample = StorageGuardPathStatus {
            label: observed.label.to_string(),
            path: observed.path.to_string_lossy().to_string(),
            mount_point: disk.mount_point.clone(),
            free_bytes: disk.available_bytes.saturating_add(reserve_bonus),
            total_bytes: disk.total_bytes,
        };
        let should_replace = match active.as_ref() {
            Some(current) => sample.free_bytes < current.free_bytes,
            None => true,
        };
        if should_replace {
            active = Some(sample);
        }
    }

    let level = match active.as_ref().map(|path| path.free_bytes) {
        Some(bytes) if bytes <= EMERGENCY_FREE_BYTES => StorageGuardLevel::Emergency,
        Some(bytes) if bytes <= WARNING_FREE_BYTES => StorageGuardLevel::Warning,
        _ => StorageGuardLevel::Normal,
    };

    StorageAssessment {
        status: StorageGuardStatus {
            level,
            warning_threshold_bytes: WARNING_FREE_BYTES,
            emergency_threshold_bytes: EMERGENCY_FREE_BYTES,
            reserve_bytes: RESERVE_BYTES,
            reserve_file_active,
            active,
            updated_at: Utc::now().to_rfc3339(),
        },
        reserve_mount_point,
    }
}

async fn collect_observed_paths(
    state: &Arc<AppState>,
    extra_paths: &[PathBuf],
) -> Vec<ObservedPath> {
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
    paths: &mut Vec<ObservedPath>,
    seen: &mut HashSet<PathBuf>,
    label: &'static str,
    path: PathBuf,
) {
    if !seen.insert(path.clone()) {
        return;
    }
    paths.push(ObservedPath { label, path });
}

async fn running_session_workdirs(state: &Arc<AppState>) -> Vec<PathBuf> {
    let mut workdirs = Vec::new();
    for session_id in state.list_running_sessions().await {
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

fn ensure_reserve_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create storage reserve directory {}",
                parent.to_string_lossy()
            )
        })?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| {
            format!(
                "failed to open storage reserve file {}",
                path.to_string_lossy()
            )
        })?;
    file.allocate(RESERVE_BYTES).with_context(|| {
        format!(
            "failed to allocate {} bytes for storage reserve file {}",
            RESERVE_BYTES,
            path.to_string_lossy()
        )
    })?;
    file.set_len(RESERVE_BYTES).with_context(|| {
        format!(
            "failed to set storage reserve file size for {}",
            path.to_string_lossy()
        )
    })?;
    Ok(())
}

fn release_reserve_file(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(path).with_context(|| {
        format!(
            "failed to remove storage reserve file {}",
            path.to_string_lossy()
        )
    })
}

fn format_path_label(path: &StorageGuardPathStatus) -> String {
    if path.mount_point == path.path {
        return path.label.clone();
    }
    format!("{} ({})", path.label, path.mount_point)
}

impl AppState {
    pub fn storage_guard_snapshot(&self) -> StorageGuardStatus {
        self.core.storage_guard.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tempfile::tempdir;
    use tokio::sync::mpsc;

    use ctx_core::ids::SessionId;
    use ctx_store::StoreManager;

    use super::*;
    use crate::daemon::AppState;

    fn disk(mount_point: &str, available_bytes: u64, total_bytes: u64) -> DiskSnapshot {
        DiskSnapshot {
            name: mount_point.to_string(),
            mount_point: mount_point.to_string(),
            total_bytes,
            available_bytes,
            file_system: "apfs".to_string(),
        }
    }

    async fn app_state_for_test() -> Arc<AppState> {
        let data_root = tempdir().expect("data root");
        let stores = StoreManager::open(data_root.path()).await.expect("stores");
        Arc::new(AppState::new(
            data_root.path().to_path_buf(),
            stores,
            HashMap::new(),
            "http://127.0.0.1:4399".to_string(),
            None,
        ))
    }

    #[test]
    fn storage_assessment_uses_reserve_bytes_for_data_root_mount() {
        let data_root = PathBuf::from("/ctx-data");
        let observed = vec![
            ObservedPath {
                label: "CTX data root",
                path: data_root.clone(),
            },
            ObservedPath {
                label: "temp storage",
                path: PathBuf::from("/tmp"),
            },
        ];
        let assessment = build_storage_assessment(
            &data_root,
            &observed,
            &[disk("/", 768 * MIB, 10 * GIB)],
            true,
        );

        let active = assessment.status.active.expect("active path");
        assert_eq!(active.free_bytes, 768 * MIB + RESERVE_BYTES);
        assert_eq!(assessment.status.level, StorageGuardLevel::Warning);
    }

    #[test]
    fn storage_assessment_prefers_lowest_free_mount() {
        let data_root = PathBuf::from("/ctx-data");
        let observed = vec![
            ObservedPath {
                label: "CTX data root",
                path: data_root.clone(),
            },
            ObservedPath {
                label: "active worktree",
                path: PathBuf::from("/Volumes/work/repo"),
            },
        ];
        let assessment = build_storage_assessment(
            &data_root,
            &observed,
            &[
                disk("/", 20 * GIB, 100 * GIB),
                disk("/Volumes/work", 900 * MIB, 100 * GIB),
            ],
            false,
        );

        let active = assessment.status.active.expect("active path");
        assert_eq!(active.label, "active worktree");
        assert_eq!(active.mount_point, "/Volumes/work");
        assert_eq!(assessment.status.level, StorageGuardLevel::Emergency);
    }

    #[test]
    fn storage_exhaustion_detection_matches_expected_errors() {
        assert!(is_storage_exhaustion_error("database or disk is full"));
        assert!(is_storage_exhaustion_error(
            "No space left on device (os error 28)"
        ));
        assert!(!is_storage_exhaustion_error("permission denied"));
    }

    #[tokio::test]
    async fn preflight_blocks_turn_start_during_emergency() {
        let state = app_state_for_test().await;
        state.core.storage_guard.publish(StorageGuardStatus {
            level: StorageGuardLevel::Emergency,
            reserve_file_active: false,
            active: Some(StorageGuardPathStatus {
                label: "CTX data root".to_string(),
                path: state.core.data_root.to_string_lossy().to_string(),
                mount_point: "/".to_string(),
                free_bytes: 900 * MIB,
                total_bytes: 10 * GIB,
            }),
            ..StorageGuardStatus::default()
        });

        let err = preflight_turn_start(&state, &state.core.data_root)
            .await
            .expect_err("preflight should fail");
        assert!(err.to_string().contains("Storage is critically low"));
    }

    #[tokio::test]
    async fn dispatches_storage_emergency_interrupts_to_running_sessions() {
        let state = app_state_for_test().await;
        let session_id = SessionId(uuid::Uuid::new_v4());
        let (tx, mut rx) = mpsc::channel(1);

        {
            let mut schedulers = state.sessions.schedulers.lock().await;
            schedulers.insert(session_id, crate::daemon::TimedEntry::new(tx));
        }
        state.set_running(session_id, true).await;

        let interrupted = dispatch_storage_emergency_interrupt(&state, session_id).await;
        assert!(interrupted);
        let received = rx.recv().await.expect("storage emergency command");
        assert!(matches!(received, SchedulerCommand::StorageEmergency));
    }
}
