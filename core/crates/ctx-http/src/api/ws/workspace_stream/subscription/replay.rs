use super::super::lifecycle::queue_workspace_stream_reset;
use super::*;
use ctx_workspace_active_snapshot::ResolvedWorkspaceActiveSessionSubscription;
use std::collections::HashSet;

mod cursor;
mod session;

use cursor::{resume_replay_cursor, skip_replay_sessions_after_snapshot};
use session::replay_workspace_session;

fn replay_cursor_after_live_progress(
    subscriptions: &HashMap<SessionId, SessionCursor>,
    session_id: SessionId,
    requested_cursor: SessionReplayCursor,
) -> Option<SessionReplayCursor> {
    subscriptions
        .get(&session_id)
        .map(|cursor| cursor.last_sent.cover(requested_cursor))
}

fn workspace_stream_event_blocks_pending_replay(
    event: &WorkspaceActiveSnapshotEvent,
    pending_replay_sessions: &HashSet<SessionId>,
    active_task_sessions: &HashMap<TaskId, SessionId>,
) -> bool {
    match event {
        WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { task, .. } => {
            pending_replay_sessions.contains(&primary_session_id_for_active_task(task))
        }
        WorkspaceActiveSnapshotEvent::ActiveTaskDelete { task_id, .. } => active_task_sessions
            .get(task_id)
            .is_some_and(|session_id| pending_replay_sessions.contains(session_id)),
        WorkspaceActiveSnapshotEvent::TaskDelta { delta, .. } => delta
            .task
            .primary_session_id
            .is_some_and(|session_id| pending_replay_sessions.contains(&session_id)),
        WorkspaceActiveSnapshotEvent::SessionSummary { summary, .. } => {
            pending_replay_sessions.contains(&summary.session.id)
        }
        WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. } => {
            pending_replay_sessions.contains(&delta.session_id)
        }
        WorkspaceActiveSnapshotEvent::SessionRemoved { session_id, .. }
        | WorkspaceActiveSnapshotEvent::SessionGap { session_id, .. } => {
            pending_replay_sessions.contains(session_id)
        }
        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
            pending_replay_sessions.contains(&delta.session_id)
        }
        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
            pending_replay_sessions.contains(&head.session.id)
        }
        WorkspaceActiveSnapshotEvent::Ready { .. }
        | WorkspaceActiveSnapshotEvent::WorktreeBootstrap { .. }
        | WorkspaceActiveSnapshotEvent::ArchivedTaskUpsert { .. }
        | WorkspaceActiveSnapshotEvent::ArchivedTaskDelete { .. } => false,
    }
}

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
        let Some(replay_cursor) = replay_cursor_after_live_progress(
            &runtime.subscriptions,
            session_id,
            requested_replay_cursor,
        ) else {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn replay_cursor(last_event_seq: i64, projection_rev: i64) -> SessionReplayCursor {
        SessionReplayCursor {
            last_event_seq,
            projection_rev,
        }
    }

    #[test]
    fn workspace_stream_event_blocks_pending_replay_for_session_events() {
        let pending_session_id = SessionId::new();
        let other_session_id = SessionId::new();
        let pending_task_id = TaskId::new();
        let pending = HashSet::from([pending_session_id]);
        let active_task_sessions = HashMap::from([(pending_task_id, pending_session_id)]);

        assert!(workspace_stream_event_blocks_pending_replay(
            &WorkspaceActiveSnapshotEvent::SessionGap {
                workspace_id: WorkspaceId::new(),
                snapshot_rev: 1,
                session_id: pending_session_id,
                after_seq: 3,
                reason: None,
                seed_follows: false,
            },
            &pending,
            &HashMap::new(),
        ));
        assert!(workspace_stream_event_blocks_pending_replay(
            &WorkspaceActiveSnapshotEvent::ActiveTaskDelete {
                workspace_id: WorkspaceId::new(),
                snapshot_rev: 1,
                task_id: pending_task_id,
            },
            &pending,
            &active_task_sessions,
        ));
        assert!(!workspace_stream_event_blocks_pending_replay(
            &WorkspaceActiveSnapshotEvent::SessionGap {
                workspace_id: WorkspaceId::new(),
                snapshot_rev: 1,
                session_id: other_session_id,
                after_seq: 3,
                reason: None,
                seed_follows: false,
            },
            &pending,
            &HashMap::new(),
        ));
        assert!(!workspace_stream_event_blocks_pending_replay(
            &WorkspaceActiveSnapshotEvent::Ready {
                workspace_id: WorkspaceId::new(),
                snapshot_rev: 1,
                archived_rev: 0,
            },
            &pending,
            &HashMap::new(),
        ));
    }

    #[test]
    fn replay_cursor_after_live_progress_starts_after_live_cursor_and_skips_removed_sessions() {
        let live_session_id = SessionId::new();
        let removed_session_id = SessionId::new();
        let subscriptions = HashMap::from([(
            live_session_id,
            SessionCursor {
                last_sent: replay_cursor(15, 16),
            },
        )]);

        assert_eq!(
            replay_cursor_after_live_progress(
                &subscriptions,
                live_session_id,
                replay_cursor(10, 12),
            ),
            Some(replay_cursor(15, 16)),
        );
        assert_eq!(
            replay_cursor_after_live_progress(
                &subscriptions,
                live_session_id,
                replay_cursor(20, 12),
            ),
            Some(replay_cursor(20, 16)),
        );
        assert_eq!(
            replay_cursor_after_live_progress(
                &subscriptions,
                removed_session_id,
                replay_cursor(10, 12),
            ),
            None,
        );
    }
}
