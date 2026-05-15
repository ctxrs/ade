use super::*;

#[tokio::test]
async fn scoped_mcp_token_revokes_prior_token_for_same_session_scope() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, Some("daemon-secret".to_string()));
    let session_id = SessionId::new();
    let workspace_id = WorkspaceId::new();
    let worktree_id = WorktreeId::new();
    let stale_token = state
        .issue_provider_session_mcp_token(session_id, workspace_id, worktree_id)
        .await;
    let fresh_token = state
        .issue_provider_session_mcp_token(session_id, workspace_id, worktree_id)
        .await;
    let app = test_router(&state);

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/mcp/sessions/{}/list_agents", session_id.0))
        .header("authorization", format!("Bearer {stale_token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/mcp/sessions/{}/list_agents", session_id.0))
        .header("authorization", format!("Bearer {fresh_token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_ne!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn scoped_mcp_token_can_be_revoked_exactly() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, Some("daemon-secret".to_string()));
    let session_id = SessionId::new();
    let token = state
        .issue_provider_session_mcp_token(session_id, WorkspaceId::new(), WorktreeId::new())
        .await;
    let app = test_router(&state);

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/mcp/sessions/{}/list_agents", session_id.0))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_ne!(res.status(), StatusCode::UNAUTHORIZED);

    assert!(state.revoke_provider_session_mcp_token(&token).await);

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/mcp/sessions/{}/list_agents", session_id.0))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
