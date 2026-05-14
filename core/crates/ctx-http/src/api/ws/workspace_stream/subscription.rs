use super::lifecycle::{clear_runtime_queues, queue_workspace_stream_reset};
use super::*;
use ctx_workspace_active_snapshot::ResolvedWorkspaceActiveSessionSubscription;

mod replay;
#[cfg(test)]
mod tests;

use replay::{replay_workspace_stream_subscriptions, WorkspaceStreamReplayRequest};

fn workspace_subscription_fingerprint(
    include_initial_snapshot: bool,
    resolved_sessions: &[ResolvedWorkspaceActiveSessionSubscription],
    next_state: &WorkspaceActiveSubscriptionState,
) -> String {
    let mut sessions = resolved_sessions
        .iter()
        .map(|subscription| {
            let replay = match subscription.replay {
                ResolvedWorkspaceActiveSessionReplay::Reset => "reset".to_string(),
                ResolvedWorkspaceActiveSessionReplay::Resume {
                    after_seq,
                    after_projection_rev,
                } => format!("resume:{after_seq}:{after_projection_rev}"),
            };
            format!(
                "{}:{:?}:{}",
                subscription.session_id.0, subscription.intent, replay
            )
        })
        .collect::<Vec<_>>();
    sessions.sort();
    let mut foreground = next_state
        .foreground_session_ids
        .as_ref()
        .map(|ids| ids.iter().map(|id| id.0.to_string()).collect::<Vec<_>>())
        .unwrap_or_default();
    foreground.sort();
    format!(
        "heads={};active={};foreground={};sessions={}",
        include_initial_snapshot,
        next_state.active_scope,
        foreground.join(","),
        sessions.join("|")
    )
}

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
    state: &WorkspaceStreamHandle,
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
    state.activate_workspace_merge_queue(workspace_id).await;
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
    let fingerprint = workspace_subscription_fingerprint(
        include_initial_snapshot,
        &resolved_sessions,
        &next_state,
    );
    if runtime.last_subscription_fingerprint.as_deref() == Some(fingerprint.as_str()) {
        return Ok(());
    }
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
        let requested = SessionReplayCursor {
            last_event_seq: after_seq.max(0),
            projection_rev: after_projection_rev.max(0),
        };
        let last_sent = existing_replay_cursors
            .get(&subscription.session_id)
            .copied()
            .map(|existing| existing.cover(requested))
            .unwrap_or(requested);
        provisional_subscriptions.insert(subscription.session_id, SessionCursor { last_sent });
    }

    clear_runtime_queues(runtime).await;
    runtime.reset_queued = false;
    runtime.send_control.clear_disconnect_after_flush();
    runtime.subscriptions = provisional_subscriptions;
    runtime.subscription_state = next_state.clone();
    sync_workspace_stream_session_pins(
        state,
        previous_subscription_ids,
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
            .workspace_active_heads(workspace_id)
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
    runtime.last_subscription_fingerprint = Some(fingerprint);
    Ok(())
}
