use super::*;

use std::collections::HashMap;
use std::sync::Arc;

use ctx_store::StoreManager;
use ctx_update_service::route_contract::MaintenanceRouteErrorKind;

async fn test_state() -> (tempfile::TempDir, Arc<DaemonState>) {
    test_state_with_shutdown_token(None).await
}

async fn test_state_with_shutdown_token(
    local_shutdown_token: Option<String>,
) -> (tempfile::TempDir, Arc<DaemonState>) {
    let data_dir = tempfile::tempdir().expect("create tempdir");
    let stores = StoreManager::open(data_dir.path())
        .await
        .expect("open stores");
    let mut state = DaemonState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:0".to_string(),
        None,
    );
    state.core.local_shutdown_token = local_shutdown_token;
    (data_dir, Arc::new(state))
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
async fn begin_update_drain_route_requires_confirm() {
    let (_data_dir, state) = test_state().await;
    let handle = crate::daemon::DaemonHandle::new(state).execution();

    let error = handle
        .begin_update_drain_for_route(BeginUpdateDrainRouteRequest::new(false, None, None))
        .await
        .expect_err("confirm is required");

    assert_eq!(error.kind(), MaintenanceRouteErrorKind::BadRequest);
    assert_eq!(error.message(), "confirm required");
}

#[tokio::test]
async fn begin_update_drain_route_defaults_reason_and_owner() {
    let (_data_dir, state) = test_state().await;
    let handle = crate::daemon::DaemonHandle::new(Arc::clone(&state)).execution();

    let result = handle
        .begin_update_drain_for_route(BeginUpdateDrainRouteRequest::new(
            true,
            Some("  ".to_string()),
            Some("".to_string()),
        ))
        .await
        .expect("idle daemon should acquire update drain");

    assert!(result.acquired);
    assert_eq!(
        post_message_update_drain_reason(&state).await.as_deref(),
        Some("daemon_update")
    );
}

#[tokio::test]
async fn begin_update_drain_route_maps_existing_drain_to_conflict() {
    let (_data_dir, state) = test_state().await;
    let handle = crate::daemon::DaemonHandle::new(Arc::clone(&state)).execution();
    begin_update_drain(&state, "existing".to_string(), "unit_test".to_string())
        .await
        .expect("acquire initial drain");

    let error = handle
        .begin_update_drain_for_route(BeginUpdateDrainRouteRequest::new(true, None, None))
        .await
        .expect_err("second drain should conflict");

    assert_eq!(error.kind(), MaintenanceRouteErrorKind::Conflict);
    assert_eq!(error.message(), "daemon update drain already active");
}

#[tokio::test]
async fn release_update_drain_route_requires_confirm() {
    let (_data_dir, state) = test_state().await;
    let handle = crate::daemon::DaemonHandle::new(state).execution();

    let error = handle
        .release_update_drain_for_route(ReleaseUpdateDrainRouteRequest::new(false))
        .await
        .expect_err("confirm is required");

    assert_eq!(error.kind(), MaintenanceRouteErrorKind::BadRequest);
    assert_eq!(error.message(), "confirm required");
}

#[tokio::test]
async fn shutdown_route_rejects_missing_or_invalid_local_token() {
    let (_data_dir, state) = test_state_with_shutdown_token(Some("secret".to_string())).await;
    let handle = crate::daemon::DaemonHandle::new(state).execution();

    for token in [None, Some("wrong".to_string())] {
        let error = handle
            .request_daemon_shutdown_for_route(
                ShutdownDaemonRouteRequest::new(true, None).with_supplied_shutdown_token(token),
            )
            .await
            .expect_err("valid local shutdown token is required");

        assert_eq!(error.kind(), MaintenanceRouteErrorKind::Forbidden);
        assert_eq!(error.message(), "local desktop shutdown token required");
    }
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
