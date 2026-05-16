use super::fixtures::{
    create_workspace_session, create_workspace_worktree, create_worktree_for_workspace, session_id,
    test_state,
};
use super::*;
use std::collections::{HashMap, HashSet};

use crate::daemon::DaemonHandle;
use chrono::Utc;
use ctx_core::ids::{TaskId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, SessionActivityState, SessionHeadSnapshot, SessionHeadWindow,
    SessionMetadata, SessionStatus, WorkspaceActiveHeadBatch, WorkspaceActivePage,
    WorkspaceActiveSnapshot, WorkspaceActiveSnapshotClientMessage,
    WorkspaceActiveSnapshotSessionIntent, WorkspaceActiveSnapshotSessionReplay,
    WorkspaceActiveSnapshotSessionSubscription,
};
use ctx_workspace_active_snapshot::{
    ResolvedWorkspaceActiveSessionReplay, ResolvedWorkspaceActiveSessionSubscription,
    ResolvedWorkspaceActiveSubscriptions, SessionReplayCursor, WorkspaceActiveSubscriptionState,
};
use std::sync::Arc;

fn cursor(last_event_seq: i64, projection_rev: i64) -> SessionReplayCursor {
    SessionReplayCursor {
        last_event_seq,
        projection_rev,
    }
}

fn test_head(
    workspace_id: ctx_core::ids::WorkspaceId,
    session_id: ctx_core::ids::SessionId,
    last_event_seq: i64,
    projection_rev: i64,
) -> SessionHeadSnapshot {
    SessionHeadSnapshot {
        session: SessionMetadata {
            id: session_id,
            task_id: TaskId::new(),
            workspace_id,
            worktree_id: WorktreeId::new(),
            execution_environment: ExecutionEnvironment::Host,
            parent_session_id: None,
            relationship: None,
            provider_id: "fake".to_string(),
            model_id: "fake-model".to_string(),
            reasoning_effort: None,
            title: "test".to_string(),
            agent_role: "assistant".to_string(),
            status: SessionStatus::Active,
            provider_session_ref: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        turns: Vec::new(),
        tool_summaries: Vec::new(),
        events: Vec::new(),
        messages: Vec::new(),
        last_event_seq,
        projection_rev,
        state_rev: projection_rev,
        activity: SessionActivityState::default(),
        has_more_turns: false,
        history_cursor: None,
        has_more_history: false,
        summary_checkpoint: None,
        head_window: SessionHeadWindow::default(),
    }
}

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

#[test]
fn snapshot_read_model_active_head_cursors_use_delivered_heads() {
    let workspace_id = ctx_core::ids::WorkspaceId::new();
    let session_id = ctx_core::ids::SessionId::new();
    let read_model = WorkspaceStreamSnapshotReadModel {
        active_snapshot: WorkspaceActiveSnapshot {
            workspace_id,
            snapshot_rev: 500,
            archived_rev: 0,
            active: WorkspaceActivePage {
                tasks: Vec::new(),
                total_count: 0,
            },
        },
        active_heads: WorkspaceActiveHeadBatch {
            workspace_id,
            snapshot_rev: 400,
            heads: vec![test_head(workspace_id, session_id, 17, 23)],
        },
    };

    let cursors = active_head_cursors_from_snapshot_read_model(&read_model);

    assert_eq!(
        cursors.get(&session_id).copied(),
        Some(cursor(17, 23)),
        "replay cursor seeding must use the delivered active-head batch, not a later read",
    );
}

#[test]
fn resume_replay_cursor_planning_preserves_live_coverage_semantics() {
    assert_eq!(
        plan_resume_replay_cursor(Some(cursor(15, 16)), 10, 12),
        WorkspaceStreamResumeReplayCursorPlan::Replay {
            cursor: cursor(15, 16),
        },
    );
    assert_eq!(
        plan_resume_replay_cursor(Some(cursor(15, 16)), 20, 0),
        WorkspaceStreamResumeReplayCursorPlan::Replay {
            cursor: cursor(20, i64::MAX),
        },
        "zero projection revision requests must preserve the prior i64::MAX fallback",
    );
    assert_eq!(
        plan_resume_replay_cursor(None, 10, 12),
        WorkspaceStreamResumeReplayCursorPlan::NoReplayRequired,
        "the existing no-live-cursor path skips replay work",
    );
}

#[tokio::test]
async fn head_only_cursor_uses_snapshot_cursor_or_current_tail_fallback() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state(root.path()).await;
    let (workspace_id, session_id) = create_workspace_session(&state, root.path()).await;

    let from_snapshot = head_only_snapshot_cursor(
        &state,
        workspace_id,
        session_id,
        Some(cursor(4, 5)),
        Some(cursor(10, 3)),
        true,
    )
    .await;
    assert_eq!(from_snapshot, cursor(10, 5));

    let from_current_tail = head_only_snapshot_cursor(
        &state,
        workspace_id,
        session_id,
        Some(cursor(4, 5)),
        Some(cursor(10, 3)),
        false,
    )
    .await;
    assert_eq!(
        from_current_tail,
        cursor(4, 5),
        "when no initial snapshot was queued, the daemon tail cursor is covered by the live cursor",
    );
}

#[tokio::test]
async fn active_task_subscription_cursor_reads_daemon_tail() {
    let root = tempfile::tempdir().unwrap();
    let state = test_state(root.path()).await;
    let (workspace_id, session_id) = create_workspace_session(&state, root.path()).await;

    let cursor = active_task_subscription_cursor(&state, workspace_id, session_id).await;

    assert_eq!(cursor, SessionReplayCursor::default());
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
