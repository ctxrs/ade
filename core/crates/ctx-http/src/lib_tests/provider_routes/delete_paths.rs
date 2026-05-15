use super::*;

#[tokio::test]
async fn missing_provider_account_deletes_return_not_found() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, Some("daemon-secret".to_string()));
    let app = test_router(&state);

    for route in [
        "/api/providers/codex/accounts/missing",
        "/api/providers/claude-crp/accounts/missing",
        "/api/providers/gemini/accounts/missing",
        "/api/providers/qwen/accounts/missing",
        "/api/providers/kimi/accounts/missing",
        "/api/providers/amp/accounts/missing",
        "/api/providers/mistral/accounts/missing",
        "/api/providers/copilot/accounts/missing",
        "/api/providers/cursor/accounts/missing",
    ] {
        let req = Request::builder()
            .method("DELETE")
            .uri(route)
            .header(header::AUTHORIZATION, "Bearer daemon-secret")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{route}");
    }
}

#[tokio::test]
async fn missing_provider_harness_endpoint_delete_returns_not_found() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, Some("daemon-secret".to_string()));
    let app = test_router(&state);

    let req = Request::builder()
        .method("DELETE")
        .uri("/api/providers/qwen/harness_config/endpoints/missing")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
