mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ctx_core::models::{
    SessionActivityState, SessionEventType, SessionHeadDelta, SessionHeadSnapshot,
    SessionTurnStatus, VcsKind,
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
        .update_session_head(compact_head)
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
            status: SessionTurnStatus::Completed,
            start_seq: Some(1),
            end_seq: Some(1),
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
            status: SessionTurnStatus::Completed,
            start_seq: Some(1),
            end_seq: Some(1),
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
        .get_session_head(primary.id)
        .await
        .expect("store-backed read should hydrate the per-session head cache");
    assert_eq!(cached.last_event_seq, event.seq);
    assert_eq!(cached.messages.len(), 1);
    assert_eq!(cached.messages[0].content, "primary answer");
}
