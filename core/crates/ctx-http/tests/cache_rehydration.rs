mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ctx_core::models::{SessionHeadSnapshot, VcsKind};
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
