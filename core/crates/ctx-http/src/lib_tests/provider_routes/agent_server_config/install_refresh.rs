use super::*;

#[tokio::test]
async fn provider_install_surfaces_agent_server_config_errors() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    write_invalid_agent_server_config(data_dir.path());
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, None);
    let app = test_router(&state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/providers/qwen/install?target=host")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["code"].as_str(),
        Some("agent_server_config_invalid")
    );
    assert!(payload["error"]
        .as_str()
        .is_some_and(|value| value.contains("parsing agent server config")));
}

#[tokio::test]
async fn install_all_providers_surfaces_agent_server_config_errors() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    write_invalid_agent_server_config(data_dir.path());
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, None);
    let app = test_router(&state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/providers/install_all?target=host")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["code"].as_str(),
        Some("agent_server_config_invalid")
    );
    assert!(payload["error"]
        .as_str()
        .is_some_and(|value| value.contains("parsing agent server config")));
}

#[tokio::test]
async fn refresh_provider_matrix_surfaces_agent_server_config_errors() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    write_invalid_agent_server_config(data_dir.path());
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, None);
    let app = test_router(&state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/providers/matrix/refresh")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(payload["error"]
        .as_str()
        .is_some_and(|value| value.contains("failed to refresh provider statuses")));
    assert!(payload["error"]
        .as_str()
        .is_some_and(|value| value.contains("parsing agent server config")));
}
