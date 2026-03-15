mod common;

use axum::http::{Method, StatusCode};

use common::{build_state, json_request, router, setup_store};
use ctx_http::storage_guard::{StorageGuardLevel, StorageGuardPathStatus, StorageGuardStatus};

#[tokio::test]
async fn health_and_diagnostics_include_storage_guard_state() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let stores = setup_store(data_dir.path()).await;
    let state = build_state(
        data_dir.path().to_path_buf(),
        stores,
        common::fake_providers(),
        "http://127.0.0.1:0",
    );
    state.core.storage_guard.publish(StorageGuardStatus {
        level: StorageGuardLevel::Warning,
        reserve_file_active: true,
        active: Some(StorageGuardPathStatus {
            label: "CTX data root".to_string(),
            path: data_dir.path().to_string_lossy().to_string(),
            mount_point: "/".to_string(),
            free_bytes: 1_800_000_000,
            total_bytes: 10_000_000_000,
        }),
        ..StorageGuardStatus::default()
    });

    let app = router(state.clone());

    let (health_status, health): (StatusCode, serde_json::Value) =
        json_request(&app, Method::GET, "/api/health", None).await;
    assert_eq!(health_status, StatusCode::OK);
    assert_eq!(
        health
            .pointer("/storage/level")
            .and_then(serde_json::Value::as_str),
        Some("warning"),
        "expected storage warning state in health payload: {health:#?}"
    );
    assert_eq!(
        health
            .pointer("/storage/active/label")
            .and_then(serde_json::Value::as_str),
        Some("CTX data root"),
        "expected active storage path in health payload: {health:#?}"
    );

    let (diag_status, diagnostics): (StatusCode, serde_json::Value) =
        json_request(&app, Method::GET, "/api/diagnostics", None).await;
    assert_eq!(diag_status, StatusCode::OK);
    assert_eq!(
        diagnostics
            .pointer("/daemon/storage/level")
            .and_then(serde_json::Value::as_str),
        Some("warning"),
        "expected storage warning state in diagnostics payload: {diagnostics:#?}"
    );
    assert_eq!(
        diagnostics
            .pointer("/daemon/storage/active/path")
            .and_then(serde_json::Value::as_str),
        Some(data_dir.path().to_string_lossy().as_ref()),
        "expected active storage path in diagnostics payload: {diagnostics:#?}"
    );
}
