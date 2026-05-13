use std::sync::Arc;

use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::{WorkspaceActiveSnapshotEvent, WorkspaceActiveSnapshotStreamMessage};
use ctx_workspace_active_snapshot::{
    SessionReplayCursor, WorkspaceSessionReplay, WorkspaceSessionReplayItem,
};

use crate::daemon::AppState;

pub(crate) enum ReplayOutcome {
    Replay { last_sent: SessionReplayCursor },
    ResetRequired,
}

const SESSION_REPLAY_HEAD_SEED_LIMIT: u32 = 60;
// Workspace streams render a bounded session head, not an audit-log replay. If a
// subscriber misses more deltas than a recoverable head window, replaying each
// stale delta floods the per-socket head queue and delays fresh foreground
// traffic. Let replay_session_stream turn larger gaps into gap+seed recovery.
const SESSION_REPLAY_DELTA_LIMIT: usize = SESSION_REPLAY_HEAD_SEED_LIMIT as usize;

pub(crate) async fn replay_session_events<F, Fut>(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    after_cursor: SessionReplayCursor,
    list_failpoint: &'static str,
    send_failpoint: Option<&'static str>,
    mut emit: F,
) -> Result<ReplayOutcome, ()>
where
    F: FnMut(WorkspaceActiveSnapshotStreamMessage) -> Fut,
    Fut: std::future::Future<Output = Result<(), ()>>,
{
    let (snapshot_rev, _) =
        crate::daemon::workspaces::load_workspace_active_snapshot_state(state, workspace_id).await;
    if crate::fault_injection::maybe_fail(list_failpoint).is_err() {
        return Ok(ReplayOutcome::ResetRequired);
    }
    let replay = state
        .workspaces
        .workspace_active_snapshot
        .replay_session_stream(
            workspace_id,
            session_id,
            after_cursor.last_event_seq,
            after_cursor.projection_rev,
            SESSION_REPLAY_DELTA_LIMIT,
        )
        .await;
    match replay {
        WorkspaceSessionReplay::Replay {
            mut items,
            mut last_sent,
        } => {
            let saw_gap = items
                .iter()
                .any(|item| matches!(item, WorkspaceSessionReplayItem::Gap { .. }));
            let saw_seed = items
                .iter()
                .any(|item| matches!(item, WorkspaceSessionReplayItem::Seed(_)));
            if saw_gap && !saw_seed {
                let store = state.store_for_session(session_id).await.map_err(|_| ())?;
                if let Ok(Some(head)) = store
                    .get_session_head_snapshot(session_id, SESSION_REPLAY_HEAD_SEED_LIMIT, true)
                    .await
                {
                    state
                        .workspaces
                        .workspace_active_snapshot
                        .update_session_head(head.clone())
                        .await;
                    last_sent = SessionReplayCursor::from_head(&head);
                    items.push(WorkspaceSessionReplayItem::Seed(Box::new(head)));
                }
            }
            let seeded_session_ids = items
                .iter()
                .filter_map(|item| match item {
                    WorkspaceSessionReplayItem::Seed(head) => Some(head.session.id),
                    _ => None,
                })
                .collect::<Vec<_>>();
            for item in items {
                if matches!(item, WorkspaceSessionReplayItem::Delta(_)) {
                    if let Some(label) = send_failpoint {
                        crate::fault_injection::maybe_fail(label).map_err(|_| ())?;
                    }
                }
                let event = match item {
                    WorkspaceSessionReplayItem::Delta(delta) => {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                            workspace_id,
                            snapshot_rev,
                            delta,
                        }
                    }
                    WorkspaceSessionReplayItem::Gap {
                        session_id,
                        after_seq,
                        reason,
                    } => WorkspaceActiveSnapshotEvent::SessionGap {
                        workspace_id,
                        snapshot_rev,
                        session_id,
                        after_seq,
                        reason,
                        seed_follows: seeded_session_ids.contains(&session_id),
                    },
                    WorkspaceSessionReplayItem::Seed(head) => {
                        WorkspaceActiveSnapshotEvent::SessionHeadSeed {
                            workspace_id,
                            snapshot_rev,
                            head,
                        }
                    }
                };
                emit(WorkspaceActiveSnapshotStreamMessage::Event {
                    rev: 0,
                    event: Box::new(event),
                })
                .await?;
            }
            Ok(ReplayOutcome::Replay { last_sent })
        }
        WorkspaceSessionReplay::ResetRequired => Ok(ReplayOutcome::ResetRequired),
    }
}
