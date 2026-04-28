use super::*;
use crate::api::derive_browser_query_secret;

#[tokio::test]
async fn desktop_browser_query_secret_authorizes_ordinary_http_routes() {
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

    let req = Request::builder()
        .method("GET")
        .uri("/api/workspaces")
        .header("authorization", format!("Bearer {browser_secret}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let req = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("authorization", format!("Bearer {browser_secret}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "root_path": git_repo.path().to_string_lossy(),
                "name": "desktop-browser-secret-workspace"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn desktop_browser_query_secret_does_not_authorize_mcp_or_mobile_registration_routes() {
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
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("POST")
        .uri("/api/mobile/register")
        .header("authorization", format!("Bearer {browser_secret}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "device_id": "11111111-1111-1111-1111-111111111111",
                "device_label": "desktop browser secret"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
