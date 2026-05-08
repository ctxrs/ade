use super::*;

#[tokio::test]
async fn scoped_mcp_token_is_limited_to_bound_session_routes() {
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
    let session_id = SessionId::new();
    let other_session_id = SessionId::new();
    let token = crate::daemon::issue_provider_session_mcp_token(
        state.as_ref(),
        session_id,
        WorkspaceId::new(),
        WorktreeId::new(),
    )
    .await;
    let app = api::router(state.clone());

    let req = Request::builder()
        .method("GET")
        .uri("/api/workspaces")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/mcp/sessions/{}/list_agents", session_id.0))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_ne!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/mcp/sessions/{}/list_agents",
            other_session_id.0
        ))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{}/artifacts", session_id.0))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"artifacts":[]}).to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_ne!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/artifacts", session_id.0))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/artifacts", other_session_id.0))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    for (method, uri) in [
        ("POST", format!("/api/sessions/{}/interrupt", session_id.0)),
        (
            "POST",
            format!("/api/sessions/{}/authenticate", session_id.0),
        ),
        ("GET", "/api/sessions/web".to_string()),
        ("POST", "/api/sessions/web".to_string()),
        (
            "GET",
            format!(
                "/api/merge-queue/entries?workspace_id={}",
                WorkspaceId::new().0
            ),
        ),
        ("GET", "/api/blobs/blob_test".to_string()),
    ] {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{}/cancel", session_id.0))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
