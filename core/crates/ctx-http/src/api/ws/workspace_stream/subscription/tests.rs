use super::*;
use chrono::Utc;

fn cursor(last_event_seq: i64, projection_rev: i64) -> SessionCursor {
    SessionCursor {
        last_sent: SessionReplayCursor {
            last_event_seq,
            projection_rev,
        },
    }
}

fn test_head(
    workspace_id: WorkspaceId,
    session_id: SessionId,
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

#[test]
fn merge_replayed_and_live_subscriptions_keeps_live_only_sessions_and_drops_removed_sessions() {
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

    let merged = merge_replayed_and_live_subscriptions(&live_subscriptions, replayed_subscriptions);

    assert_eq!(merged.len(), 2);
    assert_eq!(
        merged
            .get(&replayed_session_id)
            .map(|subscription| subscription.last_sent),
        Some(SessionReplayCursor {
            last_event_seq: 15,
            projection_rev: 15,
        })
    );
    assert_eq!(
        merged
            .get(&live_only_session_id)
            .map(|subscription| subscription.last_sent),
        Some(SessionReplayCursor {
            last_event_seq: 7,
            projection_rev: 7,
        })
    );
    assert!(
        !merged.contains_key(&removed_session_id),
        "live subscription state must remain authoritative for removed sessions",
    );
}

#[test]
fn active_head_cursors_use_exact_snapshot_read_model_heads() {
    let workspace_id = WorkspaceId::new();
    let session_id = SessionId::new();
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
        Some(SessionReplayCursor {
            last_event_seq: 17,
            projection_rev: 23,
        }),
        "replay cursor seeding must use the delivered active-head batch, not a later read",
    );
}
