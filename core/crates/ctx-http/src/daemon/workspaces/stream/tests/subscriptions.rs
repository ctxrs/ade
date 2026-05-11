use super::fixtures::{create_workspace_session, test_state};
use super::*;
use std::collections::{HashMap, HashSet};

use ctx_core::models::{
    WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotSessionReplay,
};

#[tokio::test]
async fn subscription_resolution_filters_cross_workspace_session_references() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state(root.path()).await;
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
            scope: None,
            include_active_heads: false,
        },
        &HashMap::new(),
    )
    .await
    .unwrap();

    assert_eq!(resolved.sessions.len(), 1);
    assert_eq!(resolved.sessions[0].session_id, session_a);
    assert!(resolved.state.foreground_session_ids.is_none());
    assert_eq!(resolved.state.explicit_sessions, HashSet::from([session_a]));
}
