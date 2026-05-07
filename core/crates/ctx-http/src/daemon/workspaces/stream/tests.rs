use super::*;
use std::path::Path;

use ctx_core::models::{ExecutionEnvironment, WorkspaceActiveSnapshotSessionReplay};
use ctx_store::StoreManager;

fn session_id(value: &str) -> SessionId {
    SessionId(uuid::Uuid::parse_str(value).unwrap())
}

async fn create_workspace_session(state: &Arc<AppState>, root: &Path) -> (WorkspaceId, SessionId) {
    let workspace = state
        .global_store()
        .create_workspace(
            format!("ws-{}", uuid::Uuid::new_v4()),
            root.join(format!("ws-{}", uuid::Uuid::new_v4()))
                .to_string_lossy()
                .to_string(),
            ctx_core::models::VcsKind::Git,
        )
        .await
        .unwrap();
    let store = state.store_for_workspace(workspace.id).await.unwrap();
    let worktree = store
        .create_worktree(
            workspace.id,
            root.join(format!("worktree-{}", uuid::Uuid::new_v4()))
                .to_string_lossy()
                .to_string(),
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
            ExecutionEnvironment::Host,
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
    (workspace.id, session.id)
}

#[test]
fn replay_deserializes_subscribe_message_with_explicit_replay_modes() {
    let message = serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(
        r#"{
            "type":"subscribe",
            "sessions":[
                {
                    "session_id":"00000000-0000-0000-0000-000000000001",
                    "replay":{"mode":"auto"}
                },
                {
                    "session_id":"00000000-0000-0000-0000-000000000002",
                    "replay":{"mode":"resume","after_seq":12}
                },
                {
                    "session_id":"00000000-0000-0000-0000-000000000003",
                    "replay":{"mode":"reset"}
                }
            ]
        }"#,
    )
    .unwrap();

    let WorkspaceActiveSnapshotClientMessage::Subscribe { sessions, .. } = message;
    assert_eq!(sessions.len(), 3);
    assert!(matches!(
        sessions[0].replay,
        WorkspaceActiveSnapshotSessionReplay::Auto
    ));
    assert!(matches!(
        sessions[1].replay,
        WorkspaceActiveSnapshotSessionReplay::Resume {
            after_seq: 12,
            after_projection_rev: 0,
        }
    ));
    assert!(matches!(
        sessions[2].replay,
        WorkspaceActiveSnapshotSessionReplay::Reset
    ));
}

#[test]
fn replay_deserialization_keeps_session_ids_and_explicit_sessions_distinct() {
    let message = serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(
        r#"{
            "type":"subscribe",
            "session_ids":[
                "00000000-0000-0000-0000-000000000001",
                "00000000-0000-0000-0000-000000000002"
            ],
            "sessions":[
                {
                    "session_id":"00000000-0000-0000-0000-000000000003",
                    "replay":{"mode":"reset"}
                }
            ]
        }"#,
    )
    .unwrap();

    let WorkspaceActiveSnapshotClientMessage::Subscribe {
        session_ids,
        sessions,
        ..
    } = message;
    assert_eq!(
        session_ids,
        vec![
            session_id("00000000-0000-0000-0000-000000000001"),
            session_id("00000000-0000-0000-0000-000000000002"),
        ]
    );
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].session_id,
        session_id("00000000-0000-0000-0000-000000000003")
    );
    assert!(matches!(
        sessions[0].replay,
        WorkspaceActiveSnapshotSessionReplay::Reset
    ));
}

#[test]
fn replay_deserialization_reads_foreground_session_id() {
    let message = serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(
        r#"{
            "type":"subscribe",
            "foreground_session_id":"00000000-0000-0000-0000-000000000001"
        }"#,
    )
    .unwrap();

    let WorkspaceActiveSnapshotClientMessage::Subscribe {
        foreground_session_id,
        ..
    } = message;
    assert_eq!(
        foreground_session_id,
        Some(session_id("00000000-0000-0000-0000-000000000001"))
    );
}

#[tokio::test]
async fn subscription_resolution_filters_cross_workspace_session_references() {
    let root = tempfile::tempdir().unwrap();
    let stores = StoreManager::open(root.path()).await.unwrap();
    let state = Arc::new(AppState::new(
        root.path().to_path_buf(),
        stores,
        HashMap::new(),
        "http://127.0.0.1:4399".to_string(),
        Some("daemon-secret".to_string()),
    ));
    let (workspace_a, session_a) = create_workspace_session(&state, root.path()).await;
    let (_workspace_b, session_b) = create_workspace_session(&state, root.path()).await;

    let resolved = resolve_workspace_active_snapshot_subscriptions(
        &state,
        workspace_a,
        WorkspaceActiveSnapshotClientMessage::Subscribe {
            session_ids: vec![session_a, session_b],
            sessions: vec![
                ctx_core::models::WorkspaceActiveSnapshotSessionSubscription {
                    session_id: session_b,
                    replay: WorkspaceActiveSnapshotSessionReplay::Reset,
                },
            ],
            task_ids: Vec::new(),
            foreground_session_id: Some(session_b),
            vcs_open_session_ids: vec![session_b],
            scope: None,
            include_active_heads: false,
        },
        &HashMap::new(),
    )
    .await
    .unwrap();

    assert_eq!(resolved.sessions.len(), 1);
    assert_eq!(resolved.sessions[0].session_id, session_a);
    assert_eq!(resolved.worktree_vcs_summary_session_ids, vec![session_a]);
    assert!(resolved.worktree_vcs_open_session_ids.is_empty());
    assert!(resolved.state.vcs_open_sessions.is_empty());
    assert!(resolved.state.foreground_session_ids.is_none());
    assert_eq!(resolved.state.explicit_sessions, HashSet::from([session_a]));
}
