use super::fixtures::{
    create_workspace_session, create_workspace_worktree, create_worktree_for_workspace, session_id,
    test_state,
};
use super::*;
use std::collections::{HashMap, HashSet};

use crate::daemon::DaemonHandle;
use ctx_core::ids::{TaskId, WorktreeId};
use ctx_core::models::{
    WorkspaceActiveSnapshotClientMessage, WorkspaceActiveSnapshotSessionIntent,
    WorkspaceActiveSnapshotSessionReplay, WorkspaceActiveSnapshotSessionSubscription,
};
use ctx_workspace_active_snapshot::{
    ResolvedWorkspaceActiveSessionReplay, ResolvedWorkspaceActiveSessionSubscription,
    ResolvedWorkspaceActiveSubscriptions, SessionReplayCursor, WorkspaceActiveSubscriptionState,
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

#[test]
fn subscription_plan_derives_initial_snapshot_fingerprint_and_provisional_cursors() {
    let session_id = session_id("00000000-0000-0000-0000-000000000001");
    let task_id = TaskId::new();
    let message = WorkspaceActiveSnapshotClientMessage::Subscribe {
        session_ids: vec![session_id],
        sessions: vec![WorkspaceActiveSnapshotSessionSubscription {
            session_id,
            intent: Some(WorkspaceActiveSnapshotSessionIntent::Replay),
            replay: WorkspaceActiveSnapshotSessionReplay::Resume {
                after_seq: 10,
                after_projection_rev: 12,
            },
        }],
        task_ids: vec![task_id],
        foreground_session_id: Some(session_id),
        scope: None,
        include_active_heads: true,
    };
    let mut state = WorkspaceActiveSubscriptionState::default();
    state.active_scope = true;
    state.foreground_session_ids = Some(HashSet::from([session_id]));
    let resolved = ResolvedWorkspaceActiveSubscriptions {
        sessions: vec![ResolvedWorkspaceActiveSessionSubscription {
            session_id,
            intent: WorkspaceActiveSnapshotSessionIntent::Replay,
            replay: ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: 10,
                after_projection_rev: 12,
            },
        }],
        state,
    };
    let existing = HashMap::from([(
        session_id,
        SessionReplayCursor {
            last_event_seq: 15,
            projection_rev: 16,
        },
    )]);

    let plan = plan_workspace_stream_subscription(&message, resolved, &existing);

    assert!(plan.include_initial_snapshot);
    assert_eq!(
        plan.provisional_subscriptions.get(&session_id).copied(),
        Some(SessionReplayCursor {
            last_event_seq: 15,
            projection_rev: 16,
        }),
        "existing live cursor must cover older requested replay cursor",
    );
    assert!(
        plan.fingerprint.starts_with("heads=true;active=true;"),
        "fingerprint must use the daemon-derived include_initial_snapshot flag",
    );
    assert_eq!(plan.sessions.len(), 1);
    assert!(matches!(
        plan.sessions[0].replay,
        WorkspaceStreamSessionReplay::Resume {
            after_seq: 10,
            after_projection_rev: 12,
        }
    ));
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
