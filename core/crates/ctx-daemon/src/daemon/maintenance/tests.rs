use super::*;

use std::collections::HashMap;
use std::sync::Arc;

use ctx_store::StoreManager;

async fn test_state() -> (tempfile::TempDir, Arc<DaemonState>) {
    let data_dir = tempfile::tempdir().expect("create tempdir");
    let stores = StoreManager::open(data_dir.path())
        .await
        .expect("open stores");
    let state = Arc::new(DaemonState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:0".to_string(),
        None,
    ));
    (data_dir, state)
}

#[tokio::test]
async fn begin_update_drain_acquires_until_released() {
    let (_data_dir, state) = test_state().await;

    let activity = begin_update_drain(&state, "test_update".to_string(), "unit_test".to_string())
        .await
        .expect("idle daemon should acquire update drain");
    assert!(activity.idle);
    assert_eq!(
        post_message_update_drain_reason(&state).await.as_deref(),
        Some("test_update")
    );

    let error = begin_update_drain(&state, "second".to_string(), "unit_test".to_string())
        .await
        .expect_err("second drain should conflict");
    assert!(matches!(error, BeginUpdateDrainError::AlreadyActive));

    assert!(release_update_drain(&state).await);
    assert!(post_message_update_drain_reason(&state).await.is_none());
}

#[tokio::test]
async fn execution_reject_uses_update_drain_owner() {
    let (_data_dir, state) = test_state().await;
    begin_update_drain(&state, "test_update".to_string(), "unit_test".to_string())
        .await
        .expect("acquire drain");

    let error = reject_new_execution_during_maintenance(&state)
        .await
        .expect_err("drain should reject new execution");
    assert!(error.to_string().contains("test_update"));
}

#[tokio::test]
async fn linux_sandbox_prepare_drain_conflicts_with_existing_drain() {
    let (_data_dir, state) = test_state().await;
    begin_update_drain(&state, "test_update".to_string(), "unit_test".to_string())
        .await
        .expect("acquire drain");

    let error = match acquire_linux_sandbox_prepare_drain(&state).await {
        Ok(_) => panic!("active maintenance drain should reject sandbox prepare"),
        Err(error) => error,
    };
    assert!(matches!(error, MaintenanceDrainError::AlreadyActive));
}

#[tokio::test]
async fn linux_sandbox_prepare_drain_drop_releases_drain() {
    let (_data_dir, state) = test_state().await;
    let permit = acquire_linux_sandbox_prepare_drain(&state)
        .await
        .expect("idle daemon should acquire sandbox prepare drain");
    assert_eq!(
        post_message_update_drain_reason(&state).await.as_deref(),
        Some("linux_sandbox_runtime_prepare")
    );

    drop(permit);

    for _ in 0..20 {
        if post_message_update_drain_reason(&state).await.is_none() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("dropping maintenance drain permit should release the drain");
}
