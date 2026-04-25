use super::lifecycle::queue_workspace_stream_reset;
use super::*;
use serde_json::json;

pub(crate) async fn handle_workspace_stream_lagged(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    lagged: u64,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
) -> Result<(), ()> {
    if runtime.reset_queued {
        return Ok(());
    }
    tracing::error!(
        target: "ctx_http.ws_active_snapshot",
        workspace_id = %workspace_id.0,
        lagged,
        "{}",
        labels.lagged_log,
    );
    emit_workspace_stream_incident(
        state,
        "workspace_stream_lagged",
        workspace_id,
        &[
            ("lagged", json!(lagged)),
            ("queue_label", json!(labels.event_queue_label)),
        ],
    )
    .await;
    queue_workspace_stream_reset(state, workspace_id, runtime).await
}

pub(crate) async fn handle_workspace_stream_event(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    event: WorkspaceActiveSnapshotEvent,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
) -> Result<(), ()> {
    if let Some(rev) = event_snapshot_rev(&event) {
        bump_latest_snapshot_rev(&runtime.latest_snapshot_rev, rev);
    }

    if runtime.reset_queued {
        return Ok(());
    }

    let mut refresh_active_worktrees = false;
    if runtime.subscription_state.active_scope {
        match &event {
            WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { task, .. } => {
                let session_id = primary_session_id_for_active_task(task);
                runtime
                    .subscription_state
                    .active_task_sessions
                    .insert(task.task.id, session_id);
                runtime.subscription_state.active_task_vcs_sessions.insert(
                    task.task.id,
                    primary_session_ids_for_active_task_summary(task),
                );
                refresh_active_worktrees = true;
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    runtime.subscriptions.entry(session_id)
                {
                    let last_sent = state
                        .workspaces
                        .workspace_active_snapshot
                        .session_replay_cursor(workspace_id, session_id)
                        .await;
                    entry.insert(SessionCursor { last_sent });
                }
            }
            WorkspaceActiveSnapshotEvent::ActiveTaskDelete { task_id, .. } => {
                let removed_vcs_sessions = runtime
                    .subscription_state
                    .active_task_vcs_sessions
                    .remove(task_id);
                if let Some(session_id) = runtime
                    .subscription_state
                    .active_task_sessions
                    .remove(task_id)
                {
                    refresh_active_worktrees = true;
                    let still_active = runtime
                        .subscription_state
                        .active_task_sessions
                        .values()
                        .any(|id| *id == session_id);
                    if !still_active
                        && !runtime
                            .subscription_state
                            .explicit_sessions
                            .contains(&session_id)
                    {
                        runtime.subscriptions.remove(&session_id);
                    }
                }
                if removed_vcs_sessions.is_some() {
                    refresh_active_worktrees = true;
                }
            }
            WorkspaceActiveSnapshotEvent::TaskDelta { delta, .. }
                if matches!(delta.kind, TaskDeltaKind::Archived) =>
            {
                let removed_vcs_sessions = runtime
                    .subscription_state
                    .active_task_vcs_sessions
                    .remove(&delta.task.id);
                if let Some(session_id) = runtime
                    .subscription_state
                    .active_task_sessions
                    .remove(&delta.task.id)
                {
                    refresh_active_worktrees = true;
                    let still_active = runtime
                        .subscription_state
                        .active_task_sessions
                        .values()
                        .any(|id| *id == session_id);
                    if !still_active
                        && !runtime
                            .subscription_state
                            .explicit_sessions
                            .contains(&session_id)
                    {
                        runtime.subscriptions.remove(&session_id);
                    }
                }
                if removed_vcs_sessions.is_some() {
                    refresh_active_worktrees = true;
                }
            }
            _ => {}
        }
    }
    if refresh_active_worktrees {
        let summary_session_ids = resolve_worktree_vcs_summary_session_ids(
            runtime.subscriptions.keys().copied(),
            &runtime.subscription_state,
        );
        let open_session_ids = resolve_worktree_vcs_open_session_ids(&runtime.subscription_state);
        sync_active_worktrees(
            state,
            &mut runtime.active_worktrees,
            &mut runtime.open_worktrees,
            &summary_session_ids,
            &open_session_ids,
        )
        .await;
        refresh_worktree_vcs_for_sessions(state, &summary_session_ids, &open_session_ids).await;
    }

    match &event {
        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
            let Some(cursor) = runtime.subscriptions.get_mut(&delta.session_id) else {
                return Ok(());
            };
            if !accept_session_delta(cursor, delta) {
                return Ok(());
            }
        }
        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
            let Some(cursor) = runtime.subscriptions.get_mut(&head.session.id) else {
                return Ok(());
            };
            if !accept_session_head(cursor, head) {
                return Ok(());
            }
        }
        _ => {}
    }

    let session_id = match &event {
        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => Some(delta.session_id),
        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => Some(head.session.id),
        WorkspaceActiveSnapshotEvent::SessionGap { session_id, .. } => Some(*session_id),
        WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. } => Some(delta.session_id),
        _ => None,
    };

    match event {
        WorkspaceActiveSnapshotEvent::SessionHeadDelta {
            snapshot_rev,
            delta,
            ..
        } => {
            if !should_stream_head_delta(
                &runtime.subscription_state.active_task_sessions,
                &runtime.subscription_state.explicit_sessions,
                runtime.subscription_state.foreground_session_ids.as_ref(),
                delta.session_id,
            ) {
                return Ok(());
            }
            let Some(delta) = filter_partial_delta_for_active_tasks(
                *delta,
                &runtime.subscription_state.active_task_sessions,
                runtime.subscription_state.foreground_session_ids.as_ref(),
            ) else {
                return Ok(());
            };
            let head_buffer = if is_foreground_session(
                runtime.subscription_state.foreground_session_ids.as_ref(),
                delta.session_id,
            ) {
                &runtime.foreground_head_buffer
            } else {
                &runtime.background_head_buffer
            };
            if let Err(error) = head_buffer.push(snapshot_rev, delta).await {
                log_head_batch_push_error(labels.event_queue_label, workspace_id, &error);
                if runtime.reset_queued {
                    return Ok(());
                }
                queue_workspace_stream_reset(state, workspace_id, runtime).await?;
            }
        }
        other @ WorkspaceActiveSnapshotEvent::SessionSummaryDelta { .. } => {
            if let Err(error) = runtime.summary_buffer.push(other).await {
                log_summary_batch_push_error(labels.event_queue_label, workspace_id, &error);
                if runtime.reset_queued {
                    return Ok(());
                }
                queue_workspace_stream_reset(state, workspace_id, runtime).await?;
            }
        }
        other => {
            let target = if is_priority_control_event(
                &other,
                runtime.subscription_state.foreground_session_ids.as_ref(),
            ) {
                &runtime.priority_control
            } else {
                &runtime.control
            };
            if push_stream_message(
                target,
                workspace_id,
                session_id,
                labels.event_queue_label,
                WorkspaceActiveSnapshotStreamMessage::Event {
                    rev: 0,
                    event: Box::new(other),
                },
            )
            .await
            .is_err()
            {
                if runtime.reset_queued {
                    return Ok(());
                }
                queue_workspace_stream_reset(state, workspace_id, runtime).await?;
            }
        }
    }

    Ok(())
}
