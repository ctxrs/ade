use super::lifecycle::queue_workspace_stream_reset;
use super::*;
use serde_json::json;

mod receiver;
mod route;
mod subscriptions;

pub(crate) use receiver::{
    drain_pending_workspace_stream_receiver_burst_deferring,
    flush_deferred_workspace_stream_receiver_events, handle_workspace_stream_receiver_burst,
    take_workspace_stream_receiver_burst,
};
use route::route_workspace_stream_event;
use subscriptions::update_workspace_stream_subscriptions_for_event;

pub(crate) async fn handle_workspace_stream_lagged(
    state: &WorkspaceStreamHandle,
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
    state: &WorkspaceStreamHandle,
    workspace_id: WorkspaceId,
    event: WorkspaceActiveSnapshotEvent,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
) -> Result<(), ()> {
    if let Some(rev) = state.event_snapshot_rev(&event) {
        bump_latest_snapshot_rev(&runtime.latest_snapshot_rev, rev);
    }

    if runtime.reset_queued {
        return Ok(());
    }

    if !update_workspace_stream_subscriptions_for_event(state, workspace_id, runtime, &event).await
    {
        return Ok(());
    }

    match &event {
        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
            let Some(cursor) = runtime.subscriptions.get_mut(&delta.session_id) else {
                return Ok(());
            };
            let accepted = state.accept_session_delta_cursor(cursor.last_sent, delta);
            if !accepted.accepted {
                return Ok(());
            }
            cursor.last_sent = accepted.next_cursor;
        }
        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
            let Some(cursor) = runtime.subscriptions.get_mut(&head.session.id) else {
                return Ok(());
            };
            let accepted = state.accept_session_head_cursor(cursor.last_sent, head);
            if !accepted.accepted {
                return Ok(());
            }
            cursor.last_sent = accepted.next_cursor;
        }
        _ => {}
    }

    route_workspace_stream_event(state, workspace_id, event, runtime, labels).await
}
