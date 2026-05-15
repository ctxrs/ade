use super::*;

#[tokio::test]
async fn desktop_browser_query_secret_authorizes_ordinary_http_routes() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, Some("daemon-secret".to_string()));
    let app = test_router(&state);
    let browser_secret = derive_browser_query_secret("daemon-secret");

    let req = Request::builder()
        .method("GET")
        .uri("/api/workspaces")
        .header("authorization", format!("Bearer {browser_secret}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn desktop_browser_query_secret_rejects_wrong_secret() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, Some("daemon-secret".to_string()));
    let app = test_router(&state);
    let wrong_browser_secret = derive_browser_query_secret("wrong-secret");

    let req = Request::builder()
        .method("GET")
        .uri("/api/workspaces")
        .header("authorization", format!("Bearer {wrong_browser_secret}"))
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn desktop_browser_query_secret_authorizes_owner_wide_api_routes() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, Some("daemon-secret".to_string()));
    let app = test_router(&state);
    let browser_secret = derive_browser_query_secret("daemon-secret");

    let req = Request::builder()
        .method("GET")
        .uri("/api/mcp/sessions/11111111-1111-1111-1111-111111111111/list_agents")
        .header("authorization", format!("Bearer {browser_secret}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_ne!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("GET")
        .uri("/api/mobile/access/status")
        .header("authorization", format!("Bearer {browser_secret}"))
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_ne!(res.status(), StatusCode::UNAUTHORIZED);
}
