use super::*;

#[tokio::test]
async fn mcp_context_endpoint_requires_scoped_mcp_token() {
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
    let workspace_id = WorkspaceId::new();
    let worktree_id = WorktreeId::new();
    let token = crate::daemon::issue_provider_session_mcp_token_with_capabilities(
        state.as_ref(),
        session_id,
        workspace_id,
        worktree_id,
        crate::daemon::McpAuthCapabilities::provider_turn_default(),
    )
    .await;
    let app = api::router(state);

    let req = Request::builder()
        .method("GET")
        .uri("/api/mcp/context")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        body.get("session_id").and_then(|value| value.as_str()),
        Some(session_id.0.to_string().as_str())
    );
    assert_eq!(
        body.get("workspace_id").and_then(|value| value.as_str()),
        Some(workspace_id.0.to_string().as_str())
    );
    assert_eq!(
        body.get("worktree_id").and_then(|value| value.as_str()),
        Some(worktree_id.0.to_string().as_str())
    );
    let capabilities = body
        .get("capabilities")
        .and_then(|value| value.as_array())
        .expect("capabilities");
    assert!(capabilities
        .iter()
        .any(|value| value.as_str() == Some("merge_queue_submit")));

    for bearer in ["daemon-secret", "invalid-token"] {
        let req = Request::builder()
            .method("GET")
            .uri("/api/mcp/context")
            .header("authorization", format!("Bearer {bearer}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    let req = Request::builder()
        .method("GET")
        .uri("/api/mcp/context")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_context_endpoint_requires_scoped_token_when_daemon_auth_is_disabled() {
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
        None,
    ));
    let token = crate::daemon::issue_provider_session_mcp_token(
        state.as_ref(),
        SessionId::new(),
        WorkspaceId::new(),
        WorktreeId::new(),
    )
    .await;
    let app = api::router(state);

    let req = Request::builder()
        .method("GET")
        .uri("/api/mcp/context")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("GET")
        .uri("/api/mcp/context")
        .header("authorization", "Bearer invalid-token")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("GET")
        .uri("/api/mcp/context")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
