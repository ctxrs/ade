use super::super::lifecycle::queue_workspace_stream_reset;
use super::*;
use ctx_workspace_active_snapshot::ResolvedWorkspaceActiveSessionSubscription;

pub(super) async fn replay_workspace_stream_subscriptions(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
    resolved_sessions: &[ResolvedWorkspaceActiveSessionSubscription],
    next_state: &WorkspaceActiveSubscriptionState,
    include_initial_snapshot: bool,
    active_head_cursors: &HashMap<SessionId, SessionReplayCursor>,
) -> Result<Option<HashMap<SessionId, SessionCursor>>, ()> {
    let skip_replay_sessions = skip_replay_sessions_after_snapshot(
        state,
        workspace_id,
        next_state,
        include_initial_snapshot,
        active_head_cursors,
    )
    .await;

    let mut next_map = HashMap::new();
    for sub in resolved_sessions {
        let session_id = sub.session_id;
        let ResolvedWorkspaceActiveSessionReplay::Resume {
            after_seq,
            after_projection_rev,
        } = sub.replay
        else {
            continue;
        };
        let replay_cursor = resume_replay_cursor(after_seq, after_projection_rev);
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
                        last_event_seq: last_sent.last_event_seq.max(after_seq),
                        projection_rev: last_sent.projection_rev.max(after_projection_rev),
                    },
                },
            );
            continue;
        }
        let replay = replay_workspace_session(
            state,
            workspace_id,
            session_id,
            replay_cursor,
            labels,
            next_state,
            runtime,
        )
        .await;
        match replay {
            Ok(ReplayOutcome::Replay { last_sent }) => {
                next_map.insert(session_id, SessionCursor { last_sent });
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
    Ok(Some(next_map))
}

async fn skip_replay_sessions_after_snapshot(
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

async fn replay_workspace_session(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    replay_cursor: SessionReplayCursor,
    labels: &WorkspaceStreamLabels,
    next_state: &WorkspaceActiveSubscriptionState,
    runtime: &WorkspaceStreamRuntime,
) -> Result<ReplayOutcome, ()> {
    let control = runtime.control.clone();
    let priority_control = runtime.priority_control.clone();
    let foreground_head_buffer = runtime.foreground_head_buffer.clone();
    let background_head_buffer = runtime.background_head_buffer.clone();
    let summary_buffer = runtime.summary_buffer.clone();
    let active_task_sessions = next_state.active_task_sessions.clone();
    let explicit_sessions = next_state.explicit_sessions.clone();
    let foreground_session_ids = next_state.foreground_session_ids.clone();

    replay_session_events(
        state,
        workspace_id,
        session_id,
        replay_cursor,
        labels.replay_list_metric,
        labels.replay_send_metric,
        move |event| {
            let control = control.clone();
            let priority_control = priority_control.clone();
            let foreground_head_buffer = foreground_head_buffer.clone();
            let background_head_buffer = background_head_buffer.clone();
            let summary_buffer = summary_buffer.clone();
            let active_task_sessions = active_task_sessions.clone();
            let explicit_sessions = explicit_sessions.clone();
            let foreground_session_ids = foreground_session_ids.clone();
            async move {
                match event {
                    WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => match *event {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                            snapshot_rev,
                            delta,
                            ..
                        } => {
                            if !should_stream_head_delta(
                                &active_task_sessions,
                                &explicit_sessions,
                                foreground_session_ids.as_ref(),
                                delta.session_id,
                            ) {
                                return Ok(());
                            }
                            let Some(delta) = filter_partial_delta_for_active_tasks(
                                *delta,
                                &active_task_sessions,
                                foreground_session_ids.as_ref(),
                            ) else {
                                return Ok(());
                            };
                            let head_buffer = if is_foreground_session(
                                foreground_session_ids.as_ref(),
                                delta.session_id,
                            ) {
                                &foreground_head_buffer
                            } else {
                                &background_head_buffer
                            };
                            if let Err(error) = head_buffer.push(snapshot_rev, delta).await {
                                log_head_batch_push_error(
                                    labels.replay_queue_label,
                                    workspace_id,
                                    &error,
                                );
                                return Err(());
                            }
                            Ok(())
                        }
                        other @ WorkspaceActiveSnapshotEvent::SessionSummaryDelta { .. } => {
                            summary_buffer.push(other).await.map_err(|error| {
                                log_summary_batch_push_error(
                                    labels.replay_queue_label,
                                    workspace_id,
                                    &error,
                                );
                            })?;
                            Ok(())
                        }
                        other => {
                            let target = if is_priority_control_event(
                                &other,
                                foreground_session_ids.as_ref(),
                            ) {
                                &priority_control
                            } else {
                                &control
                            };
                            push_stream_message(
                                target,
                                workspace_id,
                                Some(session_id),
                                labels.replay_queue_label,
                                WorkspaceActiveSnapshotStreamMessage::Event {
                                    rev: 0,
                                    event: Box::new(other),
                                },
                            )
                            .await
                        }
                    },
                    other => {
                        push_stream_message(
                            &control,
                            workspace_id,
                            Some(session_id),
                            labels.replay_queue_label,
                            other,
                        )
                        .await
                    }
                }
            }
        },
    )
    .await
}

fn resume_replay_cursor(after_seq: i64, after_projection_rev: i64) -> SessionReplayCursor {
    SessionReplayCursor {
        last_event_seq: after_seq,
        projection_rev: if after_projection_rev > 0 {
            after_projection_rev
        } else {
            i64::MAX
        },
    }
}
