use super::lifecycle::{clear_runtime_queues, queue_workspace_stream_reset};
use super::*;

mod replay;

use replay::{replay_workspace_stream_subscriptions, WorkspaceStreamReplayRequest};

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
    crate::daemon::merge_queue::activate_workspace_merge_queue(state, workspace_id).await;
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
        state: next_state,
    } = resolved;

    clear_runtime_queues(runtime).await;
    runtime.reset_queued = false;
    runtime.send_control.clear_disconnect_after_flush();
    let active_head_cursors = if include_initial_snapshot {
        runtime.send_control.set_hydrating();
        if queue_snapshot_payload(&runtime.control, state, workspace_id)
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

    let Some(next_map) = replay_workspace_stream_subscriptions(WorkspaceStreamReplayRequest {
        state,
        workspace_id,
        runtime,
        labels,
        resolved_sessions: &resolved_sessions,
        next_state: &next_state,
        include_initial_snapshot,
        active_head_cursors: &active_head_cursors,
    })
    .await?
    else {
        return Ok(());
    };

    sync_workspace_stream_session_pins(
        state,
        runtime.subscriptions.keys().copied(),
        next_map.keys().copied(),
    )
    .await;
    runtime.subscriptions = next_map;
    runtime.subscription_state = next_state;
    Ok(())
}
