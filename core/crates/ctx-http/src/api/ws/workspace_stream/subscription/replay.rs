use super::*;
use ctx_core::models::WorkspaceActiveSnapshotSessionIntent;
use ctx_daemon::daemon::workspaces::stream::{
    ReplayOutcome, WorkspaceStreamResumeReplayCursorPlan, WorkspaceStreamSessionReplay,
};
use std::collections::HashSet;

mod buffers;
mod live_events;
mod request;
mod reset;
mod session;

use buffers::drop_buffered_session_events_at_or_before;
use live_events::{
    drain_live_events_blocking_pending_replay, flush_replay_ready_deferred_live_events,
    replay_should_stop,
};
pub(super) use request::WorkspaceStreamReplayRequest;
use reset::queue_failed_replay_reset;
use session::replay_workspace_session;

pub(super) async fn replay_workspace_stream_subscriptions(
    request: WorkspaceStreamReplayRequest<'_>,
) -> Result<Option<HashMap<SessionId, SessionCursor>>, ()> {
    let WorkspaceStreamReplayRequest {
        state,
        workspace_id,
        runtime,
        labels,
        resolved_sessions,
        live_rx,
        include_initial_snapshot,
        active_head_cursors,
    } = request;
    let next_state = runtime.subscription_state.clone();

    let mut pending_replay_sessions = resolved_sessions
        .iter()
        .filter(|subscription| {
            subscription.intent == WorkspaceActiveSnapshotSessionIntent::Replay
                && matches!(
                    subscription.replay,
                    WorkspaceStreamSessionReplay::Resume { .. }
                )
        })
        .map(|subscription| subscription.session_id)
        .collect::<HashSet<_>>();
    let mut deferred_live_events = Vec::new();
    let mut next_map = HashMap::new();
    for sub in resolved_sessions {
        drain_live_events_blocking_pending_replay(
            state,
            workspace_id,
            live_rx,
            runtime,
            labels,
            &mut deferred_live_events,
            &pending_replay_sessions,
        )
        .await?;
        if replay_should_stop(runtime) {
            return Ok(None);
        }
        let session_id = sub.session_id;
        if sub.intent == WorkspaceActiveSnapshotSessionIntent::Head {
            next_map.insert(
                session_id,
                SessionCursor {
                    last_sent: state
                        .head_only_snapshot_cursor(
                            workspace_id,
                            session_id,
                            runtime
                                .subscriptions
                                .get(&session_id)
                                .map(|cursor| cursor.last_sent),
                            active_head_cursors.get(&session_id).copied(),
                            include_initial_snapshot,
                        )
                        .await,
                },
            );
            continue;
        }
        let WorkspaceStreamSessionReplay::Resume {
            after_seq,
            after_projection_rev,
        } = sub.replay
        else {
            continue;
        };
        let live_cursor = runtime
            .subscriptions
            .get(&session_id)
            .map(|cursor| cursor.last_sent);
        let replay_cursor =
            match state.plan_resume_replay_cursor(live_cursor, after_seq, after_projection_rev) {
                WorkspaceStreamResumeReplayCursorPlan::Replay { cursor } => cursor,
                WorkspaceStreamResumeReplayCursorPlan::NoReplayRequired => {
                    pending_replay_sessions.remove(&session_id);
                    flush_replay_ready_deferred_live_events(
                        state,
                        workspace_id,
                        runtime,
                        labels,
                        &mut deferred_live_events,
                        &pending_replay_sessions,
                    )
                    .await?;
                    if replay_should_stop(runtime) {
                        return Ok(None);
                    }
                    continue;
                }
            };
        drop_buffered_session_events_at_or_before(state, runtime, session_id, replay_cursor).await;
        let replay = replay_workspace_session(
            state,
            workspace_id,
            session_id,
            replay_cursor,
            labels,
            &next_state,
            runtime,
        )
        .await;
        match replay {
            Ok(ReplayOutcome::Replay { last_sent }) => {
                next_map.insert(session_id, SessionCursor { last_sent });
                pending_replay_sessions.remove(&session_id);
                flush_replay_ready_deferred_live_events(
                    state,
                    workspace_id,
                    runtime,
                    labels,
                    &mut deferred_live_events,
                    &pending_replay_sessions,
                )
                .await?;
                if replay_should_stop(runtime) {
                    return Ok(None);
                }
                drain_live_events_blocking_pending_replay(
                    state,
                    workspace_id,
                    live_rx,
                    runtime,
                    labels,
                    &mut deferred_live_events,
                    &pending_replay_sessions,
                )
                .await?;
                if replay_should_stop(runtime) {
                    return Ok(None);
                }
            }
            Ok(ReplayOutcome::ResetRequired) | Err(_) => {
                queue_failed_replay_reset(
                    state,
                    workspace_id,
                    session_id,
                    after_seq,
                    after_projection_rev,
                    labels,
                    runtime,
                )
                .await?;
                return Ok(None);
            }
        };
    }
    flush_replay_ready_deferred_live_events(
        state,
        workspace_id,
        runtime,
        labels,
        &mut deferred_live_events,
        &pending_replay_sessions,
    )
    .await?;
    if replay_should_stop(runtime) {
        return Ok(None);
    }
    Ok(Some(next_map))
}
