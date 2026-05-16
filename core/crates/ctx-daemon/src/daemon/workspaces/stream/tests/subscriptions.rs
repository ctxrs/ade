use super::fixtures::{
    create_workspace_session, create_workspace_worktree, create_worktree_for_workspace, test_state,
};
use super::*;
use std::collections::{HashMap, HashSet};

use crate::daemon::DaemonHandle;
use ctx_core::ids::WorktreeId;
use ctx_core::models::{
    WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotSessionReplay,
};
use std::sync::Arc;

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
                    intent: None,
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

#[tokio::test]
async fn handle_subscription_resolution_hydrates_active_snapshot_without_initial_heads() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state(root.path()).await;
    let (workspace_id, session_id) = create_workspace_session(&state, root.path()).await;
    let handle = DaemonHandle::new(Arc::clone(&state)).workspace_stream();

    handle
        .resolve_workspace_active_snapshot_subscriptions(
            workspace_id,
            WorkspaceActiveSnapshotClientMessage::Subscribe {
                session_ids: vec![session_id],
                sessions: Vec::new(),
                task_ids: Vec::new(),
                foreground_session_id: None,
                scope: None,
                include_active_heads: false,
            },
            &HashMap::new(),
        )
        .await
        .unwrap();

    let snapshot = state
        .workspaces
        .workspace_active_snapshot
        .active_snapshot(workspace_id, i64::MAX)
        .await;
    assert_eq!(
        snapshot.active.tasks.len(),
        1,
        "subscription resolution must prepare the daemon read model even without an initial snapshot",
    );
}

#[tokio::test]
async fn worktree_vcs_filter_removes_cross_workspace_duplicate_and_missing_ids() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state(root.path()).await;
    let (workspace_a, worktree_a1) = create_workspace_worktree(&state, root.path()).await;
    let worktree_a2 = create_worktree_for_workspace(&state, root.path(), workspace_a).await;
    let (_workspace_b, worktree_b) = create_workspace_worktree(&state, root.path()).await;

    let filtered = filter_workspace_worktree_ids(
        &state,
        workspace_a,
        vec![
            worktree_b.id,
            worktree_a2.id,
            WorktreeId::new(),
            worktree_a1.id,
            worktree_a1.id,
        ],
    )
    .await;

    let mut expected = vec![worktree_a1.id, worktree_a2.id];
    expected.sort_by_key(|worktree_id| worktree_id.0);
    assert_eq!(filtered, expected);
}
