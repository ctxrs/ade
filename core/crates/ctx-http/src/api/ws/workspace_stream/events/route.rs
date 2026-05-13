use super::super::lifecycle::queue_workspace_stream_reset;
use super::super::*;

pub(super) async fn route_workspace_stream_event(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    event: WorkspaceActiveSnapshotEvent,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
) -> Result<(), ()> {
    let session_id = match &event {
        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => Some(delta.session_id),
        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => Some(head.session.id),
        WorkspaceActiveSnapshotEvent::SessionGap { session_id, .. } => Some(*session_id),
        WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. } => Some(delta.session_id),
        WorkspaceActiveSnapshotEvent::SessionRemoved { session_id, .. } => Some(*session_id),
        _ => None,
    };

    match event {
        WorkspaceActiveSnapshotEvent::SessionHeadDelta {
            snapshot_rev,
            delta,
            ..
        } => route_head_delta(state, workspace_id, snapshot_rev, *delta, runtime, labels).await,
        other @ WorkspaceActiveSnapshotEvent::SessionSummaryDelta { .. } => {
            route_summary_delta(state, workspace_id, other, runtime, labels).await
        }
        other => route_control_event(state, workspace_id, session_id, other, runtime, labels).await,
    }
}

async fn route_head_delta(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    snapshot_rev: i64,
    delta: SessionHeadDelta,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
) -> Result<(), ()> {
    if !should_stream_head_delta(
        &runtime.subscription_state.active_task_sessions,
        &runtime.subscription_state.explicit_sessions,
        runtime.subscription_state.foreground_session_ids.as_ref(),
        delta.session_id,
    ) {
        return Ok(());
    }
    let Some(delta) = filter_partial_delta_for_active_tasks(
        delta,
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
    Ok(())
}

async fn route_summary_delta(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    event: WorkspaceActiveSnapshotEvent,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
) -> Result<(), ()> {
    if let Err(error) = runtime.summary_buffer.push(event).await {
        log_summary_batch_push_error(labels.event_queue_label, workspace_id, &error);
        if runtime.reset_queued {
            return Ok(());
        }
        queue_workspace_stream_reset(state, workspace_id, runtime).await?;
    }
    Ok(())
}

async fn route_control_event(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: Option<SessionId>,
    event: WorkspaceActiveSnapshotEvent,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
) -> Result<(), ()> {
    let target = if is_priority_control_event(
        &event,
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
            event: Box::new(event),
            stream_source: None,
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
    Ok(())
}
