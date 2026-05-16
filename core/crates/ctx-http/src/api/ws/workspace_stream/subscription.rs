use super::lifecycle::{clear_runtime_queues, queue_workspace_stream_reset};
use super::*;
use ctx_daemon::daemon::workspaces::stream::WorkspaceStreamSubscriptionResolutionError;

mod replay;
#[cfg(test)]
mod tests;

use replay::{replay_workspace_stream_subscriptions, WorkspaceStreamReplayRequest};

fn merge_replayed_and_live_subscriptions(
    state: &WorkspaceStreamHandle,
    live_subscriptions: &HashMap<SessionId, SessionCursor>,
    replayed_subscriptions: HashMap<SessionId, SessionCursor>,
) -> HashMap<SessionId, SessionCursor> {
    let live_cursors = live_subscriptions
        .iter()
        .map(|(session_id, cursor)| (*session_id, cursor.last_sent))
        .collect::<HashMap<_, _>>();
    let replayed_cursors = replayed_subscriptions
        .into_iter()
        .map(|(session_id, cursor)| (session_id, cursor.last_sent))
        .collect::<HashMap<_, _>>();
    state
        .merge_replayed_and_live_subscription_cursors(&live_cursors, replayed_cursors)
        .into_iter()
        .map(|(session_id, last_sent)| (session_id, SessionCursor { last_sent }))
        .collect()
}

pub(crate) async fn handle_workspace_stream_subscription(
    state: &WorkspaceStreamHandle,
    workspace_id: WorkspaceId,
    message: WorkspaceActiveSnapshotClientMessage,
    live_rx: &mut tokio::sync::broadcast::Receiver<WorkspaceActiveSnapshotEvent>,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
) -> Result<(), ()> {
    let existing_replay_cursors = runtime
        .subscriptions
        .iter()
        .map(|(session_id, cursor)| (*session_id, cursor.last_sent))
        .collect::<HashMap<_, _>>();
    let resolved = match state
        .resolve_workspace_active_snapshot_subscriptions(
            workspace_id,
            message,
            &existing_replay_cursors,
        )
        .await
    {
        Ok(next) => next,
        Err(WorkspaceStreamSubscriptionResolutionError::Hydration(error)) => {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                "workspace stream hydration failed: {error:?}"
            );
            return Err(());
        }
        Err(WorkspaceStreamSubscriptionResolutionError::Resolution) => {
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
    let include_initial_snapshot = resolved.include_initial_snapshot;
    let fingerprint = resolved.fingerprint;
    if runtime.last_subscription_fingerprint.as_deref() == Some(fingerprint.as_str()) {
        return Ok(());
    }
    let previous_subscription_ids = runtime.subscriptions.keys().copied().collect::<Vec<_>>();

    clear_runtime_queues(runtime).await;
    runtime.reset_queued = false;
    runtime.send_control.clear_disconnect_after_flush();
    runtime.subscriptions = resolved
        .provisional_subscriptions
        .iter()
        .map(|(session_id, last_sent)| {
            (
                *session_id,
                SessionCursor {
                    last_sent: *last_sent,
                },
            )
        })
        .collect();
    runtime.subscription_state = resolved.state.clone();
    sync_workspace_stream_session_pins(
        state,
        previous_subscription_ids,
        runtime.subscriptions.keys().copied(),
    )
    .await;
    let active_head_cursors = if include_initial_snapshot {
        runtime.send_control.set_hydrating();
        let read_model = if let Ok(read_model) =
            queue_snapshot_payload(&runtime.control, state, workspace_id).await
        {
            read_model
        } else {
            return Err(());
        };
        state.active_head_cursors_from_snapshot_read_model(&read_model)
    } else {
        HashMap::new()
    };

    let Some(next_map) = replay_workspace_stream_subscriptions(WorkspaceStreamReplayRequest {
        state,
        workspace_id,
        runtime,
        labels,
        resolved_sessions: &resolved.sessions,
        live_rx,
        include_initial_snapshot,
        active_head_cursors: &active_head_cursors,
    })
    .await?
    else {
        return Ok(());
    };

    let final_map = merge_replayed_and_live_subscriptions(state, &runtime.subscriptions, next_map);

    sync_workspace_stream_session_pins(
        state,
        runtime.subscriptions.keys().copied(),
        final_map.keys().copied(),
    )
    .await;
    runtime.subscriptions = final_map;
    runtime.last_subscription_fingerprint = Some(fingerprint);
    Ok(())
}
