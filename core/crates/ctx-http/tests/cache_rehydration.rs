mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ctx_core::models::{
    SessionActivityState, SessionEventType, SessionHeadDelta, SessionHeadSnapshot,
    SessionTurnStatus, VcsKind, WorkspaceActiveHeadBatch,
};
use tempfile::tempdir;

#[tokio::test]
async fn session_head_rehydrates_after_cache_eviction() {
    let temp = tempdir().unwrap();
    let stores = common::setup_store(temp.path()).await;
    let state = common::build_state(
        temp.path(),
        stores.clone(),
        common::fake_providers(),
        "http://localhost",
    );

    let workspace_root = temp.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root).await.unwrap();

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
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
    store
        .set_task_primary_session(task.id, session.id, worktree.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .unwrap();

    let baseline = store
        .get_session_head_snapshot(session.id, 60, true)
        .await
        .unwrap()
        .expect("session head snapshot");
    state
        .workspaces
        .workspace_active_snapshot
        .update_session_head(baseline.clone())
        .await;
    assert!(state
        .workspaces
        .workspace_active_snapshot
        .get_session_head(session.id)
        .await
        .is_some());

    state.cleanup_session(session.id).await;
    assert!(state
        .workspaces
        .workspace_active_snapshot
        .get_session_head(session.id)
        .await
        .is_none());

    let app = common::router(state.clone());
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/head?include_events=true&limit=60",
            session.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let (status, head): (StatusCode, SessionHeadSnapshot) = common::oneshot_json(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(head.session.id, session.id);
    assert_eq!(head.last_event_seq, baseline.last_event_seq);
}

#[tokio::test]
async fn include_events_session_heads_bypass_compact_cache() {
    let temp = tempdir().unwrap();
    let stores = common::setup_store(temp.path()).await;
    let state = common::build_state(
        temp.path(),
        stores.clone(),
        common::fake_providers(),
        "http://localhost",
    );

    let workspace_root = temp.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root).await.unwrap();

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
            "deadbeef".to_string(),
            None,
        )
        .await
        .unwrap();
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_task_index(task.id, workspace.id)
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

    store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            serde_json::json!({ "kind": "cache_boundary", "message": "persist me" }),
        )
        .await
        .unwrap();

    let full_head = store
        .get_session_head_snapshot(session.id, 60, true)
        .await
        .unwrap()
        .expect("full session head snapshot");
    assert!(
        !full_head.events.is_empty(),
        "full session head should include the persisted event tail"
    );

    let compact_head = store
        .get_active_snapshot_head(session.id)
        .await
        .unwrap()
        .expect("compact active head");
    assert!(compact_head.events.is_empty());
    state
        .workspaces
        .workspace_active_snapshot
        .update_compact_session_head(compact_head)
        .await;

    let app = common::router(state.clone());
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/head?include_events=true&limit=60",
            session.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let (status, head): (StatusCode, SessionHeadSnapshot) = common::oneshot_json(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(head.session.id, session.id);
    assert_eq!(head.last_event_seq, full_head.last_event_seq);
    assert_eq!(head.events.len(), full_head.events.len());
    assert_eq!(
        head.events.last().map(|event| event.seq),
        full_head.events.last().map(|event| event.seq)
    );
}

#[tokio::test]
async fn include_events_session_heads_use_hydrated_replay_cache_when_store_cannot_open() {
    let temp = tempdir().unwrap();
    let stores = common::setup_store(temp.path()).await;
    let state = common::build_state(
        temp.path(),
        stores.clone(),
        common::fake_providers(),
        "http://localhost",
    );

    let workspace_root = temp.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root).await.unwrap();

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
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

    store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            serde_json::json!({ "kind": "hydrated_cache", "message": "persist me" }),
        )
        .await
        .unwrap();

    let full_head = store
        .get_session_head_snapshot(session.id, 60, true)
        .await
        .unwrap()
        .expect("full replay-capable session head");
    assert!(
        !full_head.events.is_empty(),
        "full head should include persisted events for replay-capable caching"
    );
    state
        .workspaces
        .workspace_active_snapshot
        .update_session_head(full_head.clone())
        .await;

    drop(store);
    state.core.stores.evict_workspace(workspace.id).await;

    let blocked_workspace_store_dir = temp
        .path()
        .join("db")
        .join("workspaces")
        .join(workspace.id.0.to_string());
    if let Ok(metadata) = tokio::fs::metadata(&blocked_workspace_store_dir).await {
        if metadata.is_dir() {
            tokio::fs::remove_dir_all(&blocked_workspace_store_dir)
                .await
                .unwrap();
        } else {
            tokio::fs::remove_file(&blocked_workspace_store_dir)
                .await
                .unwrap();
        }
    }
    tokio::fs::create_dir_all(blocked_workspace_store_dir.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&blocked_workspace_store_dir, b"blocked workspace store")
        .await
        .unwrap();

    let app = common::router(state.clone());
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/head?include_events=true&limit=60",
            session.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let (status, head): (StatusCode, SessionHeadSnapshot) = common::oneshot_json(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(head.session.id, session.id);
    assert_eq!(head.last_event_seq, full_head.last_event_seq);
    assert_eq!(head.events.len(), full_head.events.len());
}

#[tokio::test]
async fn archiving_task_invalidates_cached_replay_session_head() {
    let temp = tempdir().unwrap();
    let stores = common::setup_store(temp.path()).await;
    let state = common::build_state(
        temp.path(),
        stores.clone(),
        common::fake_providers(),
        "http://localhost",
    );

    let repo = common::init_git_repo(&[("file.txt", "hello\n")]).await;
    let app = common::router(state.clone());
    let workspace = common::create_workspace(&app, repo.path(), "ws").await;
    let (task, session) =
        common::create_task_with_session(&app, workspace.id.0, "task", "fake", "model").await;
    let store = state.store_for_workspace(workspace.id).await.unwrap();

    let full_head = store
        .get_session_head_snapshot(session.id, 60, true)
        .await
        .unwrap()
        .expect("full replay-capable session head");
    state
        .workspaces
        .workspace_active_snapshot
        .update_session_head(full_head)
        .await;
    assert!(state
        .workspaces
        .workspace_active_snapshot
        .get_session_head(session.id)
        .await
        .is_some());

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/archive", task.id.0))
        .body(Body::empty())
        .unwrap();
    let (status, _body) = common::oneshot_bytes(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        state
            .workspaces
            .workspace_active_snapshot
            .get_session_head(session.id)
            .await
            .is_none(),
        "task archive should invalidate replay-capable cached heads for the task's sessions"
    );
}

#[tokio::test]
async fn non_primary_store_backed_head_is_purged_on_workspace_cleanup() {
    let temp = tempdir().unwrap();
    let stores = common::setup_store(temp.path()).await;
    let state = common::build_state(
        temp.path(),
        stores.clone(),
        common::fake_providers(),
        "http://localhost",
    );

    let workspace_root = temp.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root).await.unwrap();

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
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

    let head = store
        .get_session_head_snapshot(session.id, 60, false)
        .await
        .unwrap()
        .expect("store-backed session head");
    state
        .workspaces
        .workspace_active_snapshot
        .update_compact_session_head(head)
        .await;
    assert!(state
        .workspaces
        .workspace_active_snapshot
        .get_cached_session_head_for_read(session.id)
        .await
        .is_some());
    assert!(
        state
            .workspaces
            .workspace_active_snapshot
            .get_session_head(session.id)
            .await
            .is_none(),
        "event-stripped reads should not populate the replay-capable session-head cache"
    );

    drop(store);
    state.cleanup_workspace(workspace.id).await;
    state
        .global_store()
        .delete_workspace_indexes(workspace.id)
        .await
        .unwrap();
    state
        .global_store()
        .delete_workspace(workspace.id)
        .await
        .unwrap();

    assert!(state
        .workspaces
        .workspace_active_snapshot
        .get_cached_session_head_for_read(session.id)
        .await
        .is_none());

    let app = common::router(state.clone());
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/head?include_events=false&limit=60",
            session.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let (status, _body) = common::oneshot_bytes(&app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/workspaces/{}/attachments", workspace.id.0))
        .body(Body::empty())
        .unwrap();
    let (status, _body) = common::oneshot_bytes(&app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn session_read_routes_return_500_when_workspace_store_cannot_open() {
    let temp = tempdir().unwrap();
    let stores = common::setup_store(temp.path()).await;
    let state = common::build_state(
        temp.path(),
        stores.clone(),
        common::fake_providers(),
        "http://localhost",
    );

    let workspace_root = temp.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root).await.unwrap();

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
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

    drop(store);
    state.cleanup_session(session.id).await;
    state.core.stores.evict_workspace(workspace.id).await;

    let blocked_workspace_store_dir = temp
        .path()
        .join("db")
        .join("workspaces")
        .join(workspace.id.0.to_string());
    if let Ok(metadata) = tokio::fs::metadata(&blocked_workspace_store_dir).await {
        if metadata.is_dir() {
            tokio::fs::remove_dir_all(&blocked_workspace_store_dir)
                .await
                .unwrap();
        } else {
            tokio::fs::remove_file(&blocked_workspace_store_dir)
                .await
                .unwrap();
        }
    }
    tokio::fs::create_dir_all(blocked_workspace_store_dir.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&blocked_workspace_store_dir, b"blocked workspace store")
        .await
        .unwrap();

    let app = common::router(state.clone());
    let turn_id = ctx_core::ids::TurnId::new();
    let routes = [
        format!(
            "/api/sessions/{}/head?include_events=false&limit=60",
            session.id.0
        ),
        format!(
            "/api/sessions/{}/snapshot?include_events=false&limit=60",
            session.id.0
        ),
        format!("/api/sessions/{}/state", session.id.0),
        format!("/api/sessions/{}/events?tail=1", session.id.0),
        format!("/api/sessions/{}/history?limit=60", session.id.0),
        format!("/api/sessions/{}/turns/{}/tools", session.id.0, turn_id.0),
        format!("/api/workspaces/{}/attachments", workspace.id.0),
        format!("/api/workspaces/{}/tasks", workspace.id.0),
    ];
    for route in routes {
        let req = Request::builder()
            .method("GET")
            .uri(route)
            .body(Body::empty())
            .unwrap();
        let (status, _body) = common::oneshot_bytes(&app, req).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{}/messages", session.id.0))
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"content":"hello from blocked workspace store"}"#,
        ))
        .unwrap();
    let (status, _body) = common::oneshot_bytes(&app, req).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn delete_in_progress_workspace_and_session_reads_return_404() {
    let temp = tempdir().unwrap();
    let stores = common::setup_store(temp.path()).await;
    let state = common::build_state(
        temp.path(),
        stores.clone(),
        common::fake_providers(),
        "http://localhost",
    );

    let workspace_root = temp.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root).await.unwrap();

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
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

    state.core.stores.begin_workspace_delete(workspace.id).await;

    let app = common::router(state.clone());
    for route in [
        format!(
            "/api/sessions/{}/head?include_events=false&limit=60",
            session.id.0
        ),
        format!(
            "/api/sessions/{}/head?include_events=true&limit=60",
            session.id.0
        ),
        format!("/api/workspaces/{}/attachments", workspace.id.0),
        format!("/api/workspaces/{}/active_heads", workspace.id.0),
    ] {
        let req = Request::builder()
            .method("GET")
            .uri(route)
            .body(Body::empty())
            .unwrap();
        let (status, _body) = common::oneshot_bytes(&app, req).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    state
        .core
        .stores
        .finish_workspace_delete(workspace.id)
        .await;
}

#[tokio::test]
async fn include_events_false_subagent_heads_fall_back_to_store_after_cold_delta() {
    let temp = tempdir().unwrap();
    let stores = common::setup_store(temp.path()).await;
    let state = common::build_state(
        temp.path(),
        stores.clone(),
        common::fake_providers(),
        "http://localhost",
    );

    let workspace_root = temp.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root).await.unwrap();

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
            "deadbeef".to_string(),
            None,
        )
        .await
        .unwrap();
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .unwrap();
    let primary = store
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
    store
        .set_task_primary_session(task.id, primary.id, worktree.id)
        .await
        .unwrap();
    let subagent = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ctx_core::models::ExecutionEnvironment::Host,
            "fake".to_string(),
            "model".to_string(),
            "reviewer".to_string(),
            Some(primary.id),
            Some("sub_agent".to_string()),
            None,
        )
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(primary.id, workspace.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(subagent.id, workspace.id)
        .await
        .unwrap();

    let run_id = ctx_core::ids::RunId::new();
    let turn_id = ctx_core::ids::TurnId::new();
    let now = chrono::Utc::now();
    store
        .insert_session_turn(ctx_core::models::SessionTurn {
            turn_id,
            session_id: subagent.id,
            run_id: Some(run_id),
            user_message_id: None,
            status: SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            failure: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .unwrap();
    let event = store
        .append_session_event(
            subagent.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::Notice,
            serde_json::json!({"msg":"subagent durable history"}),
        )
        .await
        .unwrap();
    store
        .update_session_turn_status(
            subagent.id,
            turn_id,
            SessionTurnStatus::Completed,
            Some(event.seq),
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    store
        .insert_message(ctx_core::models::Message {
            id: ctx_core::ids::MessageId::new(),
            session_id: subagent.id,
            task_id: task.id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
            turn_sequence: Some(1),
            order_seq: None,
            role: ctx_core::models::MessageRole::Assistant,
            content: "subagent answer".to_string(),
            attachments: vec![],
            delivery: ctx_core::models::MessageDelivery::Immediate,
            delivered_at: None,
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let projection_rev = store.get_session_projection_rev(subagent.id).await.unwrap();
    let delta = SessionHeadDelta {
        session_id: subagent.id,
        last_event_seq: event.seq,
        projection_rev,
        state_rev: event.seq,
        emitted_at_ms: None,
        session: None,
        activity: Some(SessionActivityState {
            is_working: true,
            last_turn_status: Some(SessionTurnStatus::Running),
        }),
        event: None,
        turn: None,
        message: None,
        tool_summaries: Vec::new(),
    };
    state
        .workspaces
        .workspace_active_snapshot
        .publish_session_head_delta(workspace.id, &subagent, delta, true)
        .await;
    assert!(
        state
            .workspaces
            .workspace_active_snapshot
            .get_session_head(subagent.id)
            .await
            .is_none(),
        "cold subagent delta should not synthesize an in-memory head"
    );

    let app = common::router(state.clone());
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/head?limit=60", subagent.id.0))
        .body(Body::empty())
        .unwrap();
    let (status, head): (StatusCode, SessionHeadSnapshot) = common::oneshot_json(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(head.session.id, subagent.id);
    assert_eq!(head.last_event_seq, event.seq);
    assert_eq!(head.messages.len(), 1);
    assert_eq!(head.messages[0].content, "subagent answer");
}

#[tokio::test]
async fn unarchive_repopulates_active_heads_for_hydrated_workspace() {
    let temp = tempdir().unwrap();
    let stores = common::setup_store(temp.path()).await;
    let state = common::build_state(
        temp.path(),
        stores.clone(),
        common::fake_providers(),
        "http://localhost",
    );

    let workspace_root = temp.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root).await.unwrap();

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
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
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .unwrap();
    let run_id = ctx_core::ids::RunId::new();
    let turn_id = ctx_core::ids::TurnId::new();
    let now = chrono::Utc::now();
    store
        .insert_session_turn(ctx_core::models::SessionTurn {
            turn_id,
            session_id: session.id,
            run_id: Some(run_id),
            user_message_id: None,
            status: SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            failure: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .unwrap();
    let event = store
        .append_session_event(
            session.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::Notice,
            serde_json::json!({"note": "seed"}),
        )
        .await
        .unwrap();
    store
        .update_session_turn_status(
            session.id,
            turn_id,
            SessionTurnStatus::Completed,
            Some(event.seq),
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    let active_summary = store
        .get_workspace_active_task_summary(task.id)
        .await
        .unwrap()
        .expect("active task summary");
    let head = store
        .get_session_head_snapshot(session.id, 60, true)
        .await
        .unwrap()
        .expect("session head snapshot");
    state
        .workspaces
        .workspace_active_snapshot
        .hydrate_snapshot(workspace.id, 1, 0, vec![active_summary], vec![head])
        .await;

    let app = common::router(state.clone());

    let heads_req = Request::builder()
        .method("GET")
        .uri(format!("/api/workspaces/{}/active_heads", workspace.id.0))
        .body(Body::empty())
        .unwrap();
    let (heads_status, heads): (StatusCode, WorkspaceActiveHeadBatch) =
        common::oneshot_json(&app, heads_req).await;
    assert_eq!(heads_status, StatusCode::OK);
    assert_eq!(heads.heads.len(), 1);
    assert_eq!(heads.heads[0].session.id, session.id);

    let archive_req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/archive", task.id.0))
        .body(Body::empty())
        .unwrap();
    let (archive_status, _): (StatusCode, serde_json::Value) =
        common::oneshot_json(&app, archive_req).await;
    assert_eq!(archive_status, StatusCode::OK);

    let unarchive_req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/unarchive", task.id.0))
        .body(Body::empty())
        .unwrap();
    let (unarchive_status, _): (StatusCode, serde_json::Value) =
        common::oneshot_json(&app, unarchive_req).await;
    assert_eq!(unarchive_status, StatusCode::OK);
    let durable_head = state
        .store_for_session(session.id)
        .await
        .unwrap()
        .get_active_snapshot_head(session.id)
        .await
        .unwrap();
    assert!(durable_head.is_some());
    let active_task = state
        .workspaces
        .workspace_active_snapshot
        .active_task_summary(workspace.id, task.id)
        .await;
    assert!(active_task.is_some());

    let heads_req = Request::builder()
        .method("GET")
        .uri(format!("/api/workspaces/{}/active_heads", workspace.id.0))
        .body(Body::empty())
        .unwrap();
    let (heads_status, heads): (StatusCode, WorkspaceActiveHeadBatch) =
        common::oneshot_json(&app, heads_req).await;
    assert_eq!(heads_status, StatusCode::OK);
    assert_eq!(heads.heads.len(), 1);
    assert_eq!(heads.heads[0].session.id, session.id);
}

#[tokio::test]
async fn unarchive_replaces_stale_session_head_cache_before_workspace_hydration() {
    let temp = tempdir().unwrap();
    let stores = common::setup_store(temp.path()).await;
    let state = common::build_state(
        temp.path(),
        stores.clone(),
        common::fake_providers(),
        "http://localhost",
    );

    let workspace_root = temp.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root).await.unwrap();

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
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
        .upsert_workspace_task_index(task.id, workspace.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(session.id, workspace.id)
        .await
        .unwrap();
    let run_id = ctx_core::ids::RunId::new();
    let turn_id = ctx_core::ids::TurnId::new();
    let now = chrono::Utc::now();
    store
        .insert_session_turn(ctx_core::models::SessionTurn {
            turn_id,
            session_id: session.id,
            run_id: Some(run_id),
            user_message_id: None,
            status: SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            failure: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .unwrap();
    let event = store
        .append_session_event(
            session.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::Notice,
            serde_json::json!({"note": "seed"}),
        )
        .await
        .unwrap();
    store
        .update_session_turn_status(
            session.id,
            turn_id,
            SessionTurnStatus::Completed,
            Some(event.seq),
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let app = common::router(state.clone());

    let archive_req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/archive", task.id.0))
        .body(Body::empty())
        .unwrap();
    let (archive_status, _): (StatusCode, serde_json::Value) =
        common::oneshot_json(&app, archive_req).await;
    assert_eq!(archive_status, StatusCode::OK);

    let archived_head_req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/sessions/{}/head?include_events=true&limit=60",
            session.id.0
        ))
        .body(Body::empty())
        .unwrap();
    let (head_status, _head): (StatusCode, SessionHeadSnapshot) =
        common::oneshot_json(&app, archived_head_req).await;
    assert_eq!(head_status, StatusCode::OK);
    assert!(state
        .workspaces
        .workspace_active_snapshot
        .get_session_head(session.id)
        .await
        .is_some());
    assert!(
        state
            .workspaces
            .workspace_active_snapshot
            .needs_hydration(workspace.id)
            .await
    );

    let unarchive_req = Request::builder()
        .method("POST")
        .uri(format!("/api/tasks/{}/unarchive", task.id.0))
        .body(Body::empty())
        .unwrap();
    let (unarchive_status, _): (StatusCode, serde_json::Value) =
        common::oneshot_json(&app, unarchive_req).await;
    assert_eq!(unarchive_status, StatusCode::OK);

    assert!(state
        .workspaces
        .workspace_active_snapshot
        .get_session_head(session.id)
        .await
        .is_none());
    assert!(state
        .workspaces
        .workspace_active_snapshot
        .get_cached_session_head_for_read(session.id)
        .await
        .is_some());
}

#[tokio::test]
async fn include_events_false_primary_heads_fall_back_to_store_after_cold_delta() {
    let temp = tempdir().unwrap();
    let stores = common::setup_store(temp.path()).await;
    let state = common::build_state(
        temp.path(),
        stores.clone(),
        common::fake_providers(),
        "http://localhost",
    );

    let workspace_root = temp.path().join("workspace");
    tokio::fs::create_dir_all(&workspace_root).await.unwrap();

    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            workspace_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            workspace_root.to_string_lossy().to_string(),
            "deadbeef".to_string(),
            None,
        )
        .await
        .unwrap();
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .unwrap();
    let primary = store
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
    store
        .set_task_primary_session(task.id, primary.id, worktree.id)
        .await
        .unwrap();
    state
        .global_store()
        .upsert_workspace_session_index(primary.id, workspace.id)
        .await
        .unwrap();

    let run_id = ctx_core::ids::RunId::new();
    let turn_id = ctx_core::ids::TurnId::new();
    let now = chrono::Utc::now();
    store
        .insert_session_turn(ctx_core::models::SessionTurn {
            turn_id,
            session_id: primary.id,
            run_id: Some(run_id),
            user_message_id: None,
            status: SessionTurnStatus::Running,
            start_seq: Some(1),
            end_seq: None,
            started_at: now,
            updated_at: now,
            assistant_partial: None,
            thought_partial: None,
            metrics_json: None,
            failure: None,
            tool_total: 0,
            tool_pending: 0,
            tool_running: 0,
            tool_completed: 0,
            tool_failed: 0,
        })
        .await
        .unwrap();
    let event = store
        .append_session_event(
            primary.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::Notice,
            serde_json::json!({"msg":"primary durable history"}),
        )
        .await
        .unwrap();
    store
        .update_session_turn_status(
            primary.id,
            turn_id,
            SessionTurnStatus::Completed,
            Some(event.seq),
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    store
        .insert_message(ctx_core::models::Message {
            id: ctx_core::ids::MessageId::new(),
            session_id: primary.id,
            task_id: task.id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
            turn_sequence: Some(1),
            order_seq: None,
            role: ctx_core::models::MessageRole::Assistant,
            content: "primary answer".to_string(),
            attachments: vec![],
            delivery: ctx_core::models::MessageDelivery::Immediate,
            delivered_at: None,
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    let projection_rev = store.get_session_projection_rev(primary.id).await.unwrap();
    let delta = SessionHeadDelta {
        session_id: primary.id,
        last_event_seq: event.seq,
        projection_rev,
        state_rev: event.seq,
        emitted_at_ms: None,
        session: None,
        activity: Some(SessionActivityState {
            is_working: true,
            last_turn_status: Some(SessionTurnStatus::Running),
        }),
        event: None,
        turn: None,
        message: None,
        tool_summaries: Vec::new(),
    };
    state
        .workspaces
        .workspace_active_snapshot
        .publish_session_head_delta(workspace.id, &primary, delta, true)
        .await;
    assert!(
        state
            .workspaces
            .workspace_active_snapshot
            .get_session_head(primary.id)
            .await
            .is_none(),
        "cold primary delta should stay unservable until the store-backed head is loaded"
    );

    let app = common::router(state.clone());
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/head?limit=60", primary.id.0))
        .body(Body::empty())
        .unwrap();
    let (status, head): (StatusCode, SessionHeadSnapshot) = common::oneshot_json(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(head.session.id, primary.id);
    assert_eq!(head.last_event_seq, event.seq);
    assert_eq!(head.messages.len(), 1);
    assert_eq!(head.messages[0].content, "primary answer");

    let cached = state
        .workspaces
        .workspace_active_snapshot
        .get_cached_session_head_for_read(primary.id)
        .await
        .expect("store-backed read should hydrate the compact per-session head cache");
    assert_eq!(cached.last_event_seq, event.seq);
    assert_eq!(cached.messages.len(), 1);
    assert_eq!(cached.messages[0].content, "primary answer");
    assert!(
        state
            .workspaces
            .workspace_active_snapshot
            .get_session_head(primary.id)
            .await
            .is_none(),
        "include_events=false reads must not seed replay history from an event-stripped head"
    );
}
