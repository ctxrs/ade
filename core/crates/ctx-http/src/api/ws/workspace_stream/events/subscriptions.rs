use super::super::*;

#[cfg(test)]
mod tests;

pub(super) async fn update_workspace_stream_subscriptions_for_event(
    state: &WorkspaceStreamHandle,
    workspace_id: WorkspaceId,
    runtime: &mut WorkspaceStreamRuntime,
    event: &WorkspaceActiveSnapshotEvent,
) -> bool {
    let subscriptions = runtime
        .subscriptions
        .iter()
        .map(|(session_id, cursor)| (*session_id, cursor.last_sent))
        .collect::<HashMap<_, _>>();
    let application = state
        .apply_workspace_stream_subscription_event(
            workspace_id,
            runtime.subscription_state.clone(),
            subscriptions,
            event,
        )
        .await;
    runtime.subscription_state = application.state;
    runtime.subscriptions = application
        .subscriptions
        .into_iter()
        .map(|(session_id, last_sent)| (session_id, SessionCursor { last_sent }))
        .collect();
    for session_id in application.added_subscriptions {
        state.attach_session_pin(session_id).await;
    }
    for session_id in application.removed_subscriptions {
        state.detach_session_pin(session_id).await;
    }
    application.should_route
}
