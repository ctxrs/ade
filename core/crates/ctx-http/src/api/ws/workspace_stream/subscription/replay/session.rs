use super::*;
use ctx_daemon::daemon::workspaces::stream::ReplayOutcome;

pub(super) async fn replay_workspace_session(
    state: &WorkspaceStreamHandle,
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

    state
        .replay_session_events(
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
                                if let Err(error) = head_buffer
                                    .push_with_source(
                                        snapshot_rev,
                                        delta,
                                        WorkspaceActiveSnapshotStreamSource::Replay,
                                    )
                                    .await
                                {
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
                                summary_buffer
                                    .push_with_source(
                                        other,
                                        WorkspaceActiveSnapshotStreamSource::Replay,
                                    )
                                    .await
                                    .map_err(|error| {
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
                                        stream_source: Some(
                                            WorkspaceActiveSnapshotStreamSource::Replay,
                                        ),
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
