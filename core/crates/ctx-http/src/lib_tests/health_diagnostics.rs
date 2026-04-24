use super::*;

#[tokio::test]
async fn health_and_diagnostics_include_storage_guard_state() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        None,
    ));
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

    let app = api::router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/health")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
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

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/diagnostics")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let diagnostics: serde_json::Value = serde_json::from_slice(&body).unwrap();
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

#[tokio::test]
async fn unauthenticated_health_omits_sensitive_fields_when_daemon_auth_is_enabled() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let app = api::router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/health")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let health: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        health.get("auth_required").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert!(health.get("compatibility").is_some());
    assert!(
        health.get("pid").is_none(),
        "health leaked pid: {health:#?}"
    );
    assert!(
        health.get("data_root").is_none(),
        "health leaked data_root: {health:#?}"
    );
    assert!(
        health.get("daemon_url").is_none(),
        "health leaked daemon_url: {health:#?}"
    );
    assert!(
        health.get("storage").is_none(),
        "health leaked storage state: {health:#?}"
    );
}

#[tokio::test]
async fn authorized_health_keeps_sensitive_fields_when_daemon_auth_is_enabled() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let providers: HashMap<String, Arc<dyn ctx_providers::adapters::ProviderAdapter>> =
        HashMap::new();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        providers,
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let app = api::router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/health")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let health: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(health.get("pid").is_some(), "authorized health missing pid");
    assert!(
        health.get("data_root").and_then(|v| v.as_str()).is_some(),
        "authorized health missing data_root"
    );
    assert!(
        health.get("daemon_url").and_then(|v| v.as_str()).is_some(),
        "authorized health missing daemon_url"
    );
    assert!(
        health
            .pointer("/storage/level")
            .and_then(|v| v.as_str())
            .is_some(),
        "authorized health missing storage state"
    );
}
