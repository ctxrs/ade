use super::*;

pub(super) async fn skip_replay_sessions_after_snapshot(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    next_state: &WorkspaceActiveSubscriptionState,
    include_initial_snapshot: bool,
    active_head_cursors: &HashMap<SessionId, SessionReplayCursor>,
) -> HashSet<SessionId> {
    let mut skip_replay_sessions = HashSet::new();
    if include_initial_snapshot && next_state.active_scope {
        for session_id in next_state.active_task_sessions.values() {
            let Some(snapshot_cursor) = active_head_cursors.get(session_id).copied() else {
                continue;
            };
            let current_tail = state
                .workspaces
                .workspace_active_snapshot
                .session_replay_cursor(workspace_id, *session_id)
                .await;
            if current_tail <= snapshot_cursor {
                skip_replay_sessions.insert(*session_id);
            }
        }
    }
    skip_replay_sessions
}

pub(super) fn resume_replay_cursor(
    after_seq: i64,
    after_projection_rev: i64,
) -> SessionReplayCursor {
    SessionReplayCursor {
        last_event_seq: after_seq,
        projection_rev: if after_projection_rev > 0 {
            after_projection_rev
        } else {
            i64::MAX
        },
    }
}

pub(super) async fn skipped_initial_snapshot_cursor(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    replay_cursor: SessionReplayCursor,
) -> SessionCursor {
    let last_sent = state
        .workspaces
        .workspace_active_snapshot
        .session_replay_cursor(workspace_id, session_id)
        .await;
    SessionCursor {
        last_sent: SessionReplayCursor {
            last_event_seq: last_sent.last_event_seq.max(replay_cursor.last_event_seq),
            projection_rev: last_sent.projection_rev.max(replay_cursor.projection_rev),
        },
    }
}
