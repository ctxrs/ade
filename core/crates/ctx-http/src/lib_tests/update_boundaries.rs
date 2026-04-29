use super::*;

#[tokio::test]
async fn update_check_rejects_path_traversal_channel() {
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
        .uri("/api/updates/check?channel=x/../../../secret")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn daemon_shutdown_endpoint_terminalizes_running_turns_before_ack() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let mut app_state = AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    );
    app_state.core.local_shutdown_token = Some("local-shutdown-secret".to_string());
    let state = Arc::new(app_state);

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            data_dir.path().join("ws").to_string_lossy().to_string(),
            ctx_core::models::VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            data_dir.path().join("ws").to_string_lossy().to_string(),
            "deadbeef".to_string(),
            None,
        )
        .await
        .unwrap();
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .unwrap();
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ctx_core::models::ExecutionEnvironment::Host,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .unwrap();

    let turn_id = ctx_core::ids::TurnId::new();
    let now = chrono::Utc::now();
    store
        .insert_session_turn(ctx_core::models::SessionTurn {
            turn_id,
            session_id: session.id,
            run_id: Some(ctx_core::ids::RunId::new()),
            user_message_id: None,
            status: ctx_core::models::SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .unwrap();

    let app = api::router(state.clone());
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/daemon/shutdown")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .header("x-ctx-local-daemon-shutdown-token", "local-shutdown-secret")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"confirm": true}).to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let turn = store
        .get_session_turn(session.id, turn_id)
        .await
        .unwrap()
        .expect("turn exists");
    assert_eq!(
        turn.status,
        ctx_core::models::SessionTurnStatus::Interrupted
    );
}

#[tokio::test]
async fn daemon_shutdown_endpoint_requires_local_shutdown_token() {
    let data_dir = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(data_dir.path()).await.unwrap();
    let mut app_state = AppState::new(
        data_dir.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    );
    app_state.core.local_shutdown_token = Some("local-shutdown-secret".to_string());
    let app = api::router(Arc::new(app_state));

    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/daemon/shutdown")
        .header(header::AUTHORIZATION, "Bearer daemon-secret")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"confirm": true}).to_string()))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}
