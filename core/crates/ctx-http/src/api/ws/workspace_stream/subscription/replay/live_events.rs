use super::*;
use std::collections::HashSet;

pub(super) fn replay_should_stop(runtime: &WorkspaceStreamRuntime) -> bool {
    runtime.reset_queued || runtime.send_control.should_disconnect_after_flush()
}

pub(super) async fn drain_live_events_blocking_pending_replay(
    state: &WorkspaceStreamHandle,
    workspace_id: WorkspaceId,
    live_rx: &mut tokio::sync::broadcast::Receiver<WorkspaceActiveSnapshotEvent>,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
    deferred_live_events: &mut Vec<WorkspaceActiveSnapshotEvent>,
    pending_replay_sessions: &HashSet<SessionId>,
) -> Result<(), ()> {
    let active_task_sessions = runtime.subscription_state.active_task_sessions.clone();
    drain_pending_workspace_stream_receiver_burst_deferring(
        state,
        workspace_id,
        live_rx,
        runtime,
        labels,
        deferred_live_events,
        |event| {
            state.event_blocks_pending_replay(event, pending_replay_sessions, &active_task_sessions)
        },
    )
    .await
}

pub(super) async fn flush_replay_ready_deferred_live_events(
    state: &WorkspaceStreamHandle,
    workspace_id: WorkspaceId,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
    deferred_live_events: &mut Vec<WorkspaceActiveSnapshotEvent>,
    pending_replay_sessions: &HashSet<SessionId>,
) -> Result<(), ()> {
    let active_task_sessions = runtime.subscription_state.active_task_sessions.clone();
    flush_deferred_workspace_stream_receiver_events(
        state,
        workspace_id,
        runtime,
        labels,
        deferred_live_events,
        |event| {
            state.event_blocks_pending_replay(event, pending_replay_sessions, &active_task_sessions)
        },
    )
    .await
}
