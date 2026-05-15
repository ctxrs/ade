use super::*;

#[tokio::test]
async fn scoped_mcp_merge_queue_submit_is_bound_to_current_session_worktree() {
    let _serial = home_env_test_lock().lock().await;
    let home = tempfile::tempdir().unwrap();
    let _home = EnvVarGuard::set("HOME", &home.path().to_string_lossy());

    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let state = test_daemon(data_dir.path(), stores, Some("daemon-secret".to_string()));
    let session_id = SessionId::new();
    let other_session_id = SessionId::new();
    let worktree_id = WorktreeId::new();
    let other_worktree_id = WorktreeId::new();
    let token = state
        .issue_provider_session_mcp_token(session_id, WorkspaceId::new(), worktree_id)
        .await;
    let app = test_router(&state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/merge-queue/entries")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session_id.0.to_string(),
                "worktree_id": worktree_id.0.to_string()
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "provider-session MCP tokens must not include merge queue submit by default"
    );

    let token = state
        .issue_provider_session_mcp_token_with_capabilities(
            session_id,
            WorkspaceId::new(),
            worktree_id,
            ctx_mcp_auth::McpAuthCapabilities::provider_turn_default(),
        )
        .await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/merge-queue/entries")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session_id.0.to_string(),
                "worktree_id": worktree_id.0.to_string()
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_ne!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("POST")
        .uri("/api/merge-queue/entries")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": other_session_id.0.to_string(),
                "worktree_id": worktree_id.0.to_string()
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("POST")
        .uri("/api/merge-queue/entries")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "session_id": session_id.0.to_string(),
                "worktree_id": other_worktree_id.0.to_string()
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("POST")
        .uri("/api/merge-queue/entries")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "worktree_root": "/tmp/other-worktree"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
