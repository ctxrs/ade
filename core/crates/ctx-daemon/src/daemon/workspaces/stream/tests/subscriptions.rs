use super::fixtures::{
    create_workspace_session, create_workspace_worktree, create_worktree_for_workspace, session_id,
    test_state,
};
use super::*;
use std::collections::{HashMap, HashSet};

use crate::daemon::DaemonHandle;
use chrono::Utc;
use ctx_core::ids::{SessionEventId, SessionId, TaskId, TurnId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, SessionActivityState, SessionEvent, SessionEventType, SessionHeadDelta,
    SessionHeadSnapshot, SessionHeadWindow, SessionMetadata, SessionSnapshotSummary, SessionStatus,
    SessionSummaryDelta, Task, TaskDelta, TaskDeltaKind, TaskStatus, WorkspaceActiveHeadBatch,
    WorkspaceActivePage, WorkspaceActiveSnapshot, WorkspaceActiveSnapshotClientMessage,
    WorkspaceActiveSnapshotEvent, WorkspaceActiveSnapshotSessionIntent,
    WorkspaceActiveSnapshotSessionReplay, WorkspaceActiveSnapshotSessionSubscription,
    WorkspaceActiveTaskSummary, WorkspaceTaskSummary, WorktreeBootstrapNotice,
    WorktreeBootstrapStatus,
};
use ctx_workspace_active_snapshot::{
    ResolvedWorkspaceActiveSessionReplay, ResolvedWorkspaceActiveSessionSubscription,
    ResolvedWorkspaceActiveSubscriptions, SessionReplayCursor, WorkspaceActiveSubscriptionState,
};
use std::sync::Arc;

fn task(workspace_id: WorkspaceId, primary_session_id: Option<SessionId>) -> Task {
    Task {
        id: TaskId::new(),
        workspace_id,
        title: "task".to_string(),
        description: None,
        status: TaskStatus::Running,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        exec_plan_id: None,
        primary_session_id,
        primary_worktree_id: None,
        archived_at: None,
        assistant_seen_at: None,
        last_activity_at: None,
        last_assistant_message_at: None,
        has_active_session: primary_session_id.is_some(),
    }
}

fn cursor(last_event_seq: i64, projection_rev: i64) -> SessionReplayCursor {
    SessionReplayCursor {
        last_event_seq,
        projection_rev,
    }
}

fn test_session_metadata(workspace_id: WorkspaceId, session_id: SessionId) -> SessionMetadata {
    SessionMetadata {
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
    }
}

fn test_head(
    workspace_id: WorkspaceId,
    session_id: SessionId,
    last_event_seq: i64,
    projection_rev: i64,
) -> SessionHeadSnapshot {
    SessionHeadSnapshot {
        session: test_session_metadata(workspace_id, session_id),
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

fn partial_delta(session_id: SessionId) -> SessionHeadDelta {
    SessionHeadDelta {
        session_id,
        last_event_seq: 5,
        projection_rev: 7,
        state_rev: 0,
        emitted_at_ms: None,
        session: None,
        activity: None,
        event: Some(SessionEvent {
            seq: -1,
            id: SessionEventId::new(),
            session_id,
            run_id: None,
            turn_id: Some(TurnId::new()),
            event_type: SessionEventType::AssistantChunk,
            payload_json: serde_json::json!({ "content_fragment": "partial" }),
            transient: true,
            created_at: Utc::now(),
        }),
        turn: None,
        message: None,
        tool_summaries: Vec::new(),
    }
}

fn durable_delta(
    session_id: SessionId,
    last_event_seq: i64,
    projection_rev: i64,
) -> SessionHeadDelta {
    SessionHeadDelta {
        session_id,
        last_event_seq,
        projection_rev,
        state_rev: projection_rev,
        emitted_at_ms: None,
        session: None,
        activity: None,
        event: None,
        turn: None,
        message: None,
        tool_summaries: Vec::new(),
    }
}

fn session_summary(workspace_id: WorkspaceId, session_id: SessionId) -> SessionSnapshotSummary {
    SessionSnapshotSummary {
        session: test_session_metadata(workspace_id, session_id),
        last_message_at: None,
        last_message_preview: None,
        last_event_seq: None,
        projection_rev: 0,
        state_rev: 0,
        activity: SessionActivityState::default(),
        unread: None,
    }
}

fn active_task(workspace_id: WorkspaceId, session_id: SessionId) -> WorkspaceActiveTaskSummary {
    WorkspaceActiveTaskSummary {
        task: task(workspace_id, Some(session_id)),
        primary_session: session_summary(workspace_id, session_id),
        primary_session_head: None,
        sessions: Vec::new(),
        sort_at: Utc::now(),
    }
}

#[test]
fn event_routing_head_delta_applicability_preserves_subscription_rules() {
    let session_id = SessionId::new();
    let task_id = TaskId::new();
    assert!(should_stream_head_delta(
        &HashMap::new(),
        &HashSet::from([session_id]),
        None,
        session_id,
    ));
    assert!(should_stream_head_delta(
        &HashMap::new(),
        &HashSet::new(),
        Some(&HashSet::from([session_id])),
        session_id,
    ));
    assert!(should_stream_head_delta(
        &HashMap::from([(task_id, session_id)]),
        &HashSet::new(),
        None,
        session_id,
    ));
    assert!(!should_stream_head_delta(
        &HashMap::new(),
        &HashSet::new(),
        None,
        session_id,
    ));
}

#[test]
fn event_routing_partial_delta_filtering_preserves_foreground_rules() {
    let session_id = SessionId::new();

    assert!(
        filter_partial_delta_for_active_tasks(partial_delta(session_id), None).is_none(),
        "partial-only background deltas must be dropped",
    );

    let filtered = filter_partial_delta_for_active_tasks(
        partial_delta(session_id),
        Some(&HashSet::from([session_id])),
    )
    .expect("foreground partial delta should be preserved");
    assert!(filtered.event.is_some());
}

#[test]
fn event_routing_priority_control_only_matches_foreground_session_events() {
    let workspace_id = WorkspaceId::new();
    let foreground_session_id = SessionId::new();
    let background_session_id = SessionId::new();
    let foreground = HashSet::from([foreground_session_id]);
    let foreground_gap = WorkspaceActiveSnapshotEvent::SessionGap {
        workspace_id,
        snapshot_rev: 1,
        session_id: foreground_session_id,
        after_seq: 1,
        reason: Some("foreground".to_string()),
        seed_follows: false,
    };
    let background_seed = WorkspaceActiveSnapshotEvent::SessionHeadSeed {
        workspace_id,
        snapshot_rev: 1,
        head: Box::new(test_head(workspace_id, background_session_id, 1, 1)),
    };

    assert!(is_priority_control_event(
        &foreground_gap,
        Some(&foreground),
    ));
    assert!(!is_priority_control_event(
        &background_seed,
        Some(&foreground),
    ));
}

#[test]
fn event_routing_pending_replay_blockers_cover_full_event_surface() {
    let workspace_id = WorkspaceId::new();
    let session_id = SessionId::new();
    let task_id = TaskId::new();
    let pending = HashSet::from([session_id]);
    let active_task_sessions = HashMap::from([(task_id, session_id)]);
    let blocking = vec![
        WorkspaceActiveSnapshotEvent::ActiveTaskUpsert {
            workspace_id,
            snapshot_rev: 1,
            task: Box::new(active_task(workspace_id, session_id)),
        },
        WorkspaceActiveSnapshotEvent::ActiveTaskDelete {
            workspace_id,
            snapshot_rev: 1,
            task_id,
        },
        WorkspaceActiveSnapshotEvent::TaskDelta {
            workspace_id,
            snapshot_rev: 1,
            delta: Box::new(TaskDelta {
                task: task(workspace_id, Some(session_id)),
                kind: TaskDeltaKind::Updated,
            }),
        },
        WorkspaceActiveSnapshotEvent::SessionSummary {
            workspace_id,
            snapshot_rev: 1,
            summary: Box::new(session_summary(workspace_id, session_id)),
        },
        WorkspaceActiveSnapshotEvent::SessionSummaryDelta {
            workspace_id,
            snapshot_rev: 1,
            delta: Box::new(SessionSummaryDelta {
                session_id,
                task_id,
                activity: None,
                last_message_at: None,
                last_message_preview: None,
                last_event_seq: None,
                projection_rev: None,
                state_rev: None,
                emitted_at_ms: None,
            }),
        },
        WorkspaceActiveSnapshotEvent::SessionRemoved {
            workspace_id,
            snapshot_rev: 1,
            session_id,
        },
        WorkspaceActiveSnapshotEvent::SessionGap {
            workspace_id,
            snapshot_rev: 1,
            session_id,
            after_seq: 1,
            reason: None,
            seed_follows: false,
        },
        WorkspaceActiveSnapshotEvent::SessionHeadDelta {
            workspace_id,
            snapshot_rev: 1,
            delta: Box::new(partial_delta(session_id)),
        },
        WorkspaceActiveSnapshotEvent::SessionHeadSeed {
            workspace_id,
            snapshot_rev: 1,
            head: Box::new(test_head(workspace_id, session_id, 1, 1)),
        },
    ];
    for event in blocking {
        assert!(
            event_blocks_pending_replay(&event, &pending, &active_task_sessions),
            "event should block pending replay: {event:?}",
        );
    }

    let nonblocking = vec![
        WorkspaceActiveSnapshotEvent::Ready {
            workspace_id,
            snapshot_rev: 1,
            archived_rev: 0,
        },
        WorkspaceActiveSnapshotEvent::WorktreeBootstrap {
            workspace_id,
            snapshot_rev: 1,
            notice: WorktreeBootstrapNotice {
                worktree_id: WorktreeId::new(),
                worktree_root: "/tmp/worktree".to_string(),
                status: WorktreeBootstrapStatus::Success,
                started_at: Utc::now(),
                finished_at: Utc::now(),
                exit_code: Some(0),
                timeout_sec: None,
                command: None,
                script_path: None,
                log_path: None,
                log_truncated: None,
                error: None,
            },
        },
        WorkspaceActiveSnapshotEvent::ArchivedTaskUpsert {
            workspace_id,
            archived_rev: 2,
            task: Box::new(WorkspaceTaskSummary {
                task: task(workspace_id, Some(session_id)),
                provider_ids: Vec::new(),
                sessions: Vec::new(),
                sort_at: Utc::now(),
            }),
        },
        WorkspaceActiveSnapshotEvent::ArchivedTaskDelete {
            workspace_id,
            archived_rev: 2,
            task_id,
        },
    ];
    for event in nonblocking {
        assert!(
            !event_blocks_pending_replay(&event, &pending, &active_task_sessions),
            "event should not block pending replay: {event:?}",
        );
    }
}

#[test]
fn event_routing_snapshot_rev_extraction_preserves_active_snapshot_events() {
    let workspace_id = WorkspaceId::new();
    assert_eq!(
        event_snapshot_rev(&WorkspaceActiveSnapshotEvent::Ready {
            workspace_id,
            snapshot_rev: 7,
            archived_rev: 0,
        }),
        Some(7),
    );
    assert_eq!(
        event_snapshot_rev(&WorkspaceActiveSnapshotEvent::ArchivedTaskDelete {
            workspace_id,
            archived_rev: 9,
            task_id: TaskId::new(),
        }),
        None,
    );
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

#[test]
fn cursor_acceptance_preserves_transient_delta_without_advancing() {
    let current = cursor(5, 7);
    let accepted = accept_session_delta_cursor(current, &partial_delta(SessionId::new()));

    assert_eq!(
        accepted,
        WorkspaceStreamCursorAcceptance {
            accepted: true,
            next_cursor: current,
        },
    );
}

#[test]
fn cursor_acceptance_rejects_stale_durable_delta_and_advances_newer_delta() {
    let session_id = SessionId::new();
    let current = cursor(5, 7);

    assert_eq!(
        accept_session_delta_cursor(current, &durable_delta(session_id, 5, 7)),
        WorkspaceStreamCursorAcceptance {
            accepted: false,
            next_cursor: current,
        },
    );
    assert_eq!(
        accept_session_delta_cursor(current, &durable_delta(session_id, 5, 8)),
        WorkspaceStreamCursorAcceptance {
            accepted: true,
            next_cursor: cursor(5, 8),
        },
    );
}

#[test]
fn cursor_acceptance_rejects_stale_head_and_advances_newer_head() {
    let workspace_id = WorkspaceId::new();
    let session_id = SessionId::new();
    let current = cursor(5, 7);

    assert_eq!(
        accept_session_head_cursor(current, &test_head(workspace_id, session_id, 5, 7)),
        WorkspaceStreamCursorAcceptance {
            accepted: false,
            next_cursor: current,
        },
    );
    assert_eq!(
        accept_session_head_cursor(current, &test_head(workspace_id, session_id, 6, 7)),
        WorkspaceStreamCursorAcceptance {
            accepted: true,
            next_cursor: cursor(6, 7),
        },
    );
}

#[test]
fn queue_stale_drop_predicates_match_replay_cursor_ordering() {
    let session_id = SessionId::new();
    let cursor = cursor(3, 7);
    assert!(!is_session_head_delta_after_cursor(
        &durable_delta(session_id, 3, 7),
        cursor,
    ));
    assert!(is_session_head_delta_after_cursor(
        &durable_delta(session_id, 3, 8),
        cursor,
    ));

    let stale_summary = SessionSummaryDelta {
        session_id,
        task_id: TaskId::new(),
        activity: None,
        last_message_at: None,
        last_message_preview: None,
        last_event_seq: Some(3),
        projection_rev: Some(7),
        state_rev: None,
        emitted_at_ms: None,
    };
    let newer_summary = SessionSummaryDelta {
        projection_rev: Some(8),
        ..stale_summary.clone()
    };
    let uncursored_summary = SessionSummaryDelta {
        last_event_seq: None,
        ..stale_summary.clone()
    };

    assert!(!is_session_summary_delta_after_cursor(
        &stale_summary,
        cursor,
    ));
    assert!(is_session_summary_delta_after_cursor(
        &newer_summary,
        cursor,
    ));
    assert!(is_session_summary_delta_after_cursor(
        &uncursored_summary,
        cursor,
    ));
}

#[test]
fn replay_live_cursor_merge_keeps_live_authoritative() {
    let replayed_session_id = SessionId::new();
    let live_only_session_id = SessionId::new();
    let removed_session_id = SessionId::new();
    let live_subscriptions = HashMap::from([
        (replayed_session_id, cursor(15, 15)),
        (live_only_session_id, cursor(7, 7)),
    ]);
    let replayed_subscriptions = HashMap::from([
        (replayed_session_id, cursor(12, 12)),
        (removed_session_id, cursor(20, 20)),
    ]);

    let merged =
        merge_replayed_and_live_subscription_cursors(&live_subscriptions, replayed_subscriptions);

    assert_eq!(merged.len(), 2);
    assert_eq!(
        merged.get(&replayed_session_id).copied(),
        Some(cursor(15, 15))
    );
    assert_eq!(
        merged.get(&live_only_session_id).copied(),
        Some(cursor(7, 7))
    );
    assert!(
        !merged.contains_key(&removed_session_id),
        "live subscription state must remain authoritative for removed sessions",
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
