use super::lifecycle::{clear_runtime_queues, queue_workspace_stream_reset};
use super::*;
use ctx_daemon::daemon::workspaces::stream::{
    WorkspaceStreamReplayStepHook, WorkspaceStreamSubscriptionResolutionError,
};
use std::collections::HashSet;

mod replay;
#[cfg(test)]
mod tests;

use replay::{
    drain_live_events_blocking_pending_replay, replay_should_stop,
    replay_workspace_stream_subscriptions, WorkspaceStreamReplayRequest,
};

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

struct ReplayPlanningDrainHook<'a> {
    state: &'a WorkspaceStreamHandle,
    workspace_id: WorkspaceId,
    live_rx: &'a mut tokio::sync::broadcast::Receiver<WorkspaceActiveSnapshotEvent>,
    runtime: &'a mut WorkspaceStreamRuntime,
    labels: &'a WorkspaceStreamLabels,
    deferred_live_events: &'a mut Vec<WorkspaceActiveSnapshotEvent>,
}

#[async_trait::async_trait]
impl WorkspaceStreamReplayStepHook for ReplayPlanningDrainHook<'_> {
    type Error = ();

    async fn before_workspace_stream_replay_step(
        &mut self,
        pending_replay_sessions: &HashSet<SessionId>,
    ) -> Result<(), Self::Error> {
        drain_live_events_blocking_pending_replay(
            self.state,
            self.workspace_id,
            self.live_rx,
            self.runtime,
            self.labels,
            self.deferred_live_events,
            pending_replay_sessions,
        )
        .await
    }

    fn live_subscription_cursor(&self, session_id: SessionId) -> Option<SessionReplayCursor> {
        self.runtime
            .subscriptions
            .get(&session_id)
            .map(|cursor| cursor.last_sent)
    }
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
    let replay_live_cursors = runtime
        .subscriptions
        .iter()
        .map(|(session_id, cursor)| (*session_id, cursor.last_sent))
        .collect::<HashMap<_, _>>();
    let mut initial_deferred_live_events = Vec::new();
    let mut replay_planning_drain_hook = ReplayPlanningDrainHook {
        state,
        workspace_id,
        live_rx,
        runtime,
        labels,
        deferred_live_events: &mut initial_deferred_live_events,
    };
    let replay_program = state
        .plan_workspace_stream_replay_program_with_step_hook(
            workspace_id,
            &resolved.sessions,
            &replay_live_cursors,
            &active_head_cursors,
            include_initial_snapshot,
            &mut replay_planning_drain_hook,
        )
        .await?;
    drop(replay_planning_drain_hook);
    if replay_should_stop(runtime) {
        return Ok(());
    }

    let Some(next_map) = replay_workspace_stream_subscriptions(WorkspaceStreamReplayRequest {
        state,
        workspace_id,
        runtime,
        labels,
        live_rx,
        replay_program,
        initial_deferred_live_events,
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
