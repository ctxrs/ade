use super::*;

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

pub(super) async fn head_only_snapshot_cursor(
    state: &WorkspaceStreamHandle,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    live_cursor: Option<SessionReplayCursor>,
    snapshot_cursor: Option<SessionReplayCursor>,
    include_initial_snapshot: bool,
) -> SessionCursor {
    let snapshot_cursor = if include_initial_snapshot {
        snapshot_cursor
    } else {
        None
    };
    let last_sent = match snapshot_cursor {
        Some(cursor) => live_cursor.unwrap_or_default().cover(cursor),
        None => {
            let current_tail = state.session_replay_cursor(workspace_id, session_id).await;
            live_cursor.unwrap_or_default().cover(current_tail)
        }
    };
    SessionCursor { last_sent }
}
