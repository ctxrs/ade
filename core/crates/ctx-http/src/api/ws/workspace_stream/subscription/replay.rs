use super::super::lifecycle::queue_workspace_stream_reset;
use super::*;
use ctx_workspace_active_snapshot::{
    replay_cursor_after_live_progress, workspace_stream_event_blocks_pending_replay,
    ResolvedWorkspaceActiveSessionSubscription,
};
use std::collections::HashSet;

mod cursor;
mod session;

use cursor::{resume_replay_cursor, skip_replay_sessions_after_snapshot};
use session::replay_workspace_session;

pub(super) struct WorkspaceStreamReplayRequest<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) workspace_id: WorkspaceId,
    pub(super) runtime: &'a mut WorkspaceStreamRuntime,
    pub(super) labels: &'a WorkspaceStreamLabels,
    pub(super) resolved_sessions: &'a [ResolvedWorkspaceActiveSessionSubscription],
    pub(super) live_rx: &'a mut tokio::sync::broadcast::Receiver<WorkspaceActiveSnapshotEvent>,
    pub(super) include_initial_snapshot: bool,
    pub(super) active_head_cursors: &'a HashMap<SessionId, SessionReplayCursor>,
}

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

    let skip_replay_sessions = skip_replay_sessions_after_snapshot(
        state,
        workspace_id,
        &next_state,
        include_initial_snapshot,
        active_head_cursors,
    )
    .await;

    let mut pending_replay_sessions = resolved_sessions
        .iter()
        .filter(|subscription| {
            matches!(
                subscription.replay,
                ResolvedWorkspaceActiveSessionReplay::Resume { .. }
            )
        })
        .map(|subscription| subscription.session_id)
        .collect::<HashSet<_>>();
    let mut deferred_live_events = Vec::new();
    let mut next_map = HashMap::new();
    for sub in resolved_sessions {
        let active_task_sessions = runtime.subscription_state.active_task_sessions.clone();
        drain_pending_workspace_stream_receiver_burst_deferring(
            state,
            workspace_id,
            live_rx,
            runtime,
            labels,
            &mut deferred_live_events,
            |event| {
                workspace_stream_event_blocks_pending_replay(
                    event,
                    &pending_replay_sessions,
                    &active_task_sessions,
                )
            },
        )
        .await?;
        if runtime.reset_queued || runtime.send_control.should_disconnect_after_flush() {
            return Ok(None);
        }
        let session_id = sub.session_id;
        let ResolvedWorkspaceActiveSessionReplay::Resume {
            after_seq,
            after_projection_rev,
        } = sub.replay
        else {
            continue;
        };
        let requested_replay_cursor = resume_replay_cursor(after_seq, after_projection_rev);
        let live_cursor = runtime
            .subscriptions
            .get(&session_id)
            .map(|cursor| cursor.last_sent);
        let Some(replay_cursor) =
            replay_cursor_after_live_progress(live_cursor, requested_replay_cursor)
        else {
            pending_replay_sessions.remove(&session_id);
            let active_task_sessions = runtime.subscription_state.active_task_sessions.clone();
            flush_deferred_workspace_stream_receiver_events(
                state,
                workspace_id,
                runtime,
                labels,
                &mut deferred_live_events,
                |event| {
                    workspace_stream_event_blocks_pending_replay(
                        event,
                        &pending_replay_sessions,
                        &active_task_sessions,
                    )
                },
            )
            .await?;
            if runtime.reset_queued || runtime.send_control.should_disconnect_after_flush() {
                return Ok(None);
            }
            continue;
        };
        runtime
            .foreground_head_buffer
            .drop_session_deltas_at_or_before(session_id, replay_cursor)
            .await;
        runtime
            .background_head_buffer
            .drop_session_deltas_at_or_before(session_id, replay_cursor)
            .await;
        runtime
            .summary_buffer
            .drop_session_events_at_or_before(session_id, replay_cursor)
            .await;
        if include_initial_snapshot && skip_replay_sessions.contains(&session_id) {
            let last_sent = state
                .workspaces
                .workspace_active_snapshot
                .session_replay_cursor(workspace_id, session_id)
                .await;
            next_map.insert(
                session_id,
                SessionCursor {
                    last_sent: SessionReplayCursor {
                        last_event_seq: last_sent.last_event_seq.max(replay_cursor.last_event_seq),
                        projection_rev: last_sent.projection_rev.max(replay_cursor.projection_rev),
                    },
                },
            );
            pending_replay_sessions.remove(&session_id);
            let active_task_sessions = runtime.subscription_state.active_task_sessions.clone();
            flush_deferred_workspace_stream_receiver_events(
                state,
                workspace_id,
                runtime,
                labels,
                &mut deferred_live_events,
                |event| {
                    workspace_stream_event_blocks_pending_replay(
                        event,
                        &pending_replay_sessions,
                        &active_task_sessions,
                    )
                },
            )
            .await?;
            if runtime.reset_queued || runtime.send_control.should_disconnect_after_flush() {
                return Ok(None);
            }
            continue;
        }
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
                let active_task_sessions = runtime.subscription_state.active_task_sessions.clone();
                flush_deferred_workspace_stream_receiver_events(
                    state,
                    workspace_id,
                    runtime,
                    labels,
                    &mut deferred_live_events,
                    |event| {
                        workspace_stream_event_blocks_pending_replay(
                            event,
                            &pending_replay_sessions,
                            &active_task_sessions,
                        )
                    },
                )
                .await?;
                if runtime.reset_queued || runtime.send_control.should_disconnect_after_flush() {
                    return Ok(None);
                }
                let active_task_sessions = runtime.subscription_state.active_task_sessions.clone();
                drain_pending_workspace_stream_receiver_burst_deferring(
                    state,
                    workspace_id,
                    live_rx,
                    runtime,
                    labels,
                    &mut deferred_live_events,
                    |event| {
                        workspace_stream_event_blocks_pending_replay(
                            event,
                            &pending_replay_sessions,
                            &active_task_sessions,
                        )
                    },
                )
                .await?;
                if runtime.reset_queued || runtime.send_control.should_disconnect_after_flush() {
                    return Ok(None);
                }
            }
            Ok(ReplayOutcome::ResetRequired) | Err(_) => {
                tracing::error!(
                    target: "ctx_http.ws_active_snapshot",
                    workspace_id = %workspace_id.0,
                    session_id = %session_id.0,
                    after_seq,
                    after_projection_rev,
                    "{}",
                    labels.replay_failure_log,
                );
                queue_workspace_stream_reset(state, workspace_id, runtime).await?;
                return Ok(None);
            }
        };
    }
    let active_task_sessions = runtime.subscription_state.active_task_sessions.clone();
    flush_deferred_workspace_stream_receiver_events(
        state,
        workspace_id,
        runtime,
        labels,
        &mut deferred_live_events,
        |event| {
            workspace_stream_event_blocks_pending_replay(
                event,
                &pending_replay_sessions,
                &active_task_sessions,
            )
        },
    )
    .await?;
    if runtime.reset_queued || runtime.send_control.should_disconnect_after_flush() {
        return Ok(None);
    }
    Ok(Some(next_map))
}
