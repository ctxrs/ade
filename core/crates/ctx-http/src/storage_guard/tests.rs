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
fn storage_admission_denies_when_required_bytes_exceed_capacity() {
    let err = check_storage_admission(
        StorageAdmissionOperation::DiskIsolatedWorktreeMaterialization,
        2 * GIB,
        &[StorageAdmissionSample {
            label: "CTX data root".to_string(),
            path: "/ctx-data".to_string(),
            mount_point: "/".to_string(),
            free_bytes: 1200 * MIB,
            total_bytes: 20 * GIB,
        }],
    )
    .expect_err("admission should fail");
    assert_eq!(
        err.operation(),
        StorageAdmissionOperation::DiskIsolatedWorktreeMaterialization
    );
    assert!(err.to_string().contains("isolated task worktree"));
    assert!(err.to_string().contains("CTX data root"));
}

#[test]
fn storage_admission_does_not_count_reserve_bytes_before_release() {
    let err = check_storage_admission(
        StorageAdmissionOperation::DiskIsolatedWorkspaceMaterialization,
        1200 * MIB,
        &[StorageAdmissionSample {
            label: "CTX data root".to_string(),
            path: "/ctx-data".to_string(),
            mount_point: "/".to_string(),
            free_bytes: 900 * MIB,
            total_bytes: 20 * GIB,
        }],
    )
    .expect_err("inactive reserve bytes must not satisfy admission");
    assert!(err.to_string().contains("isolated workspace copy"));
    assert!(err.to_string().contains("CTX data root"));
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
async fn preflight_samples_storage_without_allocating_reserve_file() {
    let data_root = tempdir().expect("data root");
    let stores = StoreManager::open(data_root.path()).await.expect("stores");
    let state = Arc::new(AppState::new(
        data_root.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
    let _ = state.core.shutdown_tx.send(());

    preflight_turn_start(&state, &state.core.data_root)
        .await
        .expect("preflight should succeed");

    assert!(!data_root.path().join(RESERVE_FILE_NAME).exists());
    assert!(!state.storage_guard_snapshot().reserve_file_active);
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
