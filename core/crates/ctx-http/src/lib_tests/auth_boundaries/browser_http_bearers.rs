use super::*;
use crate::api::derive_browser_query_secret;

#[tokio::test]
async fn desktop_browser_query_secret_authorizes_ordinary_http_routes() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let app = api::router(state);
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
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let app = api::router(state);
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
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let app = api::router(state);
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

#[tokio::test]
async fn desktop_browser_query_secret_authorizes_high_risk_owner_routes() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let app = api::router(state);
    let browser_secret = derive_browser_query_secret("daemon-secret");
    let workspace_id = "11111111-1111-1111-1111-111111111111";

    let owner_cases = [
        (
            "POST",
            "/api/workspaces/11111111-1111-1111-1111-111111111111/merge_queue_config".to_string(),
            json!({"enabled": true}),
        ),
        (
            "POST",
            "/api/merge-queue/entries".to_string(),
            json!({"workspace_id": workspace_id}),
        ),
        (
            "POST",
            "/api/providers/codex/harness_config/endpoints".to_string(),
            json!({"base_url": "https://example.invalid"}),
        ),
        ("POST", "/api/daemon/shutdown".to_string(), json!({})),
        ("POST", "/api/updates/drain/begin".to_string(), json!({})),
        ("POST", "/api/updates/appimage/apply".to_string(), json!({})),
    ];

    for (method, uri, body) in owner_cases {
        let req = Request::builder()
            .method(method)
            .uri(uri.as_str())
            .header("authorization", format!("Bearer {browser_secret}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_ne!(res.status(), StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
}

#[tokio::test]
async fn desktop_browser_query_secret_authorizes_transition_provider_install_routes() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let app = api::router(state);
    let browser_secret = derive_browser_query_secret("daemon-secret");

    let cases = [
        ("POST", "/api/providers/not-real/install"),
        ("POST", "/api/providers/install_all"),
        ("POST", "/api/providers/install/statuses"),
        ("POST", "/api/providers/install/not-real/cancel"),
    ];

    for (method, uri) in cases {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {browser_secret}"))
            .header("content-type", "application/json")
            .body(Body::from("{"))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_ne!(res.status(), StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
}

#[tokio::test]
async fn desktop_browser_query_secret_authorizes_core_desktop_mutations() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());
    let git_repo = setup_git_repo().await;

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let app = api::router(state);
    let browser_secret = derive_browser_query_secret("daemon-secret");
    let workspace_id = "11111111-1111-1111-1111-111111111111";

    let cases = [
        (
            "POST",
            "/api/workspaces".to_string(),
            json!({
                "root_path": git_repo.path().to_string_lossy(),
                "name": "desktop-browser-secret-workspace"
            }),
        ),
        (
            "POST",
            format!("/api/workspaces/{workspace_id}/tasks"),
            json!({"title": "task"}),
        ),
        (
            "POST",
            format!("/api/workspaces/{workspace_id}/terminals"),
            json!({}),
        ),
        (
            "PUT",
            "/api/providers/codex/active-account".to_string(),
            json!({"account_id": "default"}),
        ),
        (
            "DELETE",
            "/api/providers/codex/accounts/11111111-1111-1111-1111-111111111111".to_string(),
            json!({}),
        ),
        (
            "DELETE",
            format!("/api/workspaces/{workspace_id}"),
            json!({}),
        ),
        (
            "DELETE",
            "/api/tasks/11111111-1111-1111-1111-111111111111".to_string(),
            json!({}),
        ),
    ];

    for (method, uri, body) in cases {
        let req = Request::builder()
            .method(method)
            .uri(uri.as_str())
            .header("authorization", format!("Bearer {browser_secret}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_ne!(res.status(), StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
}
