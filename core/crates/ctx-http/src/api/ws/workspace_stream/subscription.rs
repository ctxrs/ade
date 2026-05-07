use super::lifecycle::{clear_runtime_queues, queue_workspace_stream_reset};
use super::*;

pub(crate) async fn handle_workspace_stream_subscription(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    message: WorkspaceActiveSnapshotClientMessage,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
) -> Result<(), ()> {
    let include_initial_snapshot = matches!(
        &message,
        WorkspaceActiveSnapshotClientMessage::Subscribe {
            include_active_heads: true,
            ..
        }
    );
    state
        .ensure_workspace_active_snapshot_hydrated(workspace_id)
        .await
        .map_err(|error| {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                "workspace stream hydration failed: {error:?}"
            );
        })?;
    crate::merge_queue::activate_workspace_merge_queue(state, workspace_id).await;
    let existing_replay_cursors = runtime
        .subscriptions
        .iter()
        .map(|(session_id, cursor)| (*session_id, cursor.last_sent))
        .collect::<HashMap<_, _>>();
    let resolved = match resolve_workspace_active_snapshot_subscriptions(
        state,
        workspace_id,
        message,
        &existing_replay_cursors,
    )
    .await
    {
        Ok(next) => next,
        Err(_) => {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                "{}",
                labels.subscribe_resolution_log,
            );
            queue_workspace_stream_reset(state, workspace_id, runtime).await?;
            return Ok(());
        }
    };
    let ResolvedWorkspaceActiveSubscriptions {
        sessions: resolved_sessions,
        worktree_vcs_summary_session_ids,
        worktree_vcs_open_session_ids,
        state: next_state,
    } = resolved;

    clear_runtime_queues(runtime).await;
    runtime.reset_queued = false;
    runtime.send_control.clear_disconnect_after_flush();
    sync_active_worktrees(
        state,
        &mut runtime.active_worktrees,
        &mut runtime.open_worktrees,
        &worktree_vcs_summary_session_ids,
        &worktree_vcs_open_session_ids,
    )
    .await;
    let active_head_cursors = if include_initial_snapshot {
        runtime.send_control.set_hydrating();
        if queue_snapshot_payload(
            &runtime.control,
            state,
            workspace_id,
            &worktree_vcs_summary_session_ids,
            &worktree_vcs_open_session_ids,
        )
        .await
        .is_err()
        {
            return Err(());
        }
        state
            .workspaces
            .workspace_active_snapshot
            .active_heads(workspace_id)
            .await
            .heads
            .into_iter()
            .map(|head| (head.session.id, SessionReplayCursor::from_head(&head)))
            .collect::<HashMap<_, _>>()
    } else {
        HashMap::new()
    };

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

    let mut next_map = HashMap::new();
    let mut replay_failed = false;
    for sub in &resolved_sessions {
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
        let control = runtime.control.clone();
        let priority_control = runtime.priority_control.clone();
        let foreground_head_buffer = runtime.foreground_head_buffer.clone();
        let background_head_buffer = runtime.background_head_buffer.clone();
        let summary_buffer = runtime.summary_buffer.clone();
        let active_task_sessions = next_state.active_task_sessions.clone();
        let explicit_sessions = next_state.explicit_sessions.clone();
        let foreground_session_ids = next_state.foreground_session_ids.clone();

        let replay = replay_session_events(
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
                replay_failed = true;
                break;
            }
        };
    }
    if replay_failed {
        queue_workspace_stream_reset(state, workspace_id, runtime).await?;
        return Ok(());
    }
    sync_workspace_stream_session_pins(
        state,
        runtime.subscriptions.keys().copied(),
        next_map.keys().copied(),
    )
    .await;
    runtime.subscriptions = next_map;
    runtime.subscription_state = next_state;
    if seed_worktree_vcs_for_subscribe(
        &runtime.control,
        state,
        workspace_id,
        &worktree_vcs_summary_session_ids,
        if include_initial_snapshot {
            WorktreeVcsSeedMode::IncludedInSnapshot
        } else {
            WorktreeVcsSeedMode::EmitCachedEvents
        },
    )
    .await
    .is_err()
    {
        return Err(());
    }
    spawn_worktree_vcs_refresh_for_sessions(
        state.clone(),
        worktree_vcs_summary_session_ids,
        worktree_vcs_open_session_ids,
    );
    Ok(())
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
