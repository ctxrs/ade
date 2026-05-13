use super::lifecycle::{clear_runtime_queues, queue_workspace_stream_reset};
use super::*;

mod replay;
#[cfg(test)]
mod tests;

use replay::{replay_workspace_stream_subscriptions, WorkspaceStreamReplayRequest};

fn merge_replayed_and_live_subscriptions(
    live_subscriptions: &HashMap<SessionId, SessionCursor>,
    replayed_subscriptions: HashMap<SessionId, SessionCursor>,
) -> HashMap<SessionId, SessionCursor> {
    live_subscriptions
        .iter()
        .map(|(session_id, live_cursor)| {
            let last_sent = replayed_subscriptions
                .get(session_id)
                .map(|replayed_cursor| replayed_cursor.last_sent.cover(live_cursor.last_sent))
                .unwrap_or(live_cursor.last_sent);
            (*session_id, SessionCursor { last_sent })
        })
        .collect()
}

pub(crate) async fn handle_workspace_stream_subscription(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    message: WorkspaceActiveSnapshotClientMessage,
    live_rx: &mut tokio::sync::broadcast::Receiver<WorkspaceActiveSnapshotEvent>,
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
    let previous_subscription_ids = runtime.subscriptions.keys().copied().collect::<Vec<_>>();
    let mut provisional_subscriptions = HashMap::new();
    for subscription in &resolved_sessions {
        let ResolvedWorkspaceActiveSessionReplay::Resume {
            after_seq,
            after_projection_rev,
        } = subscription.replay
        else {
            continue;
        };
        provisional_subscriptions.insert(
            subscription.session_id,
            SessionCursor {
                last_sent: SessionReplayCursor {
                    last_event_seq: after_seq.max(0),
                    projection_rev: after_projection_rev.max(0),
                },
            },
        );
    }

    clear_runtime_queues(runtime).await;
    runtime.reset_queued = false;
    runtime.send_control.clear_disconnect_after_flush();
    runtime.subscriptions = provisional_subscriptions;
    runtime.subscription_state = next_state.clone();
    sync_workspace_stream_session_pins(
        state,
        previous_subscription_ids.into_iter(),
        runtime.subscriptions.keys().copied(),
    )
    .await;
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
        live_rx,
        include_initial_snapshot,
        active_head_cursors: &active_head_cursors,
    })
    .await?
    else {
        return Ok(());
    };

    let final_map = merge_replayed_and_live_subscriptions(&runtime.subscriptions, next_map);

    sync_workspace_stream_session_pins(
        state,
        runtime.subscriptions.keys().copied(),
        final_map.keys().copied(),
    )
    .await;
    runtime.subscriptions = final_map;
    Ok(())
}
