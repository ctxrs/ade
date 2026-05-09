use super::super::*;

pub(super) async fn update_workspace_stream_subscriptions_for_event(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    runtime: &mut WorkspaceStreamRuntime,
    event: &WorkspaceActiveSnapshotEvent,
) -> bool {
    if let WorkspaceActiveSnapshotEvent::SessionRemoved { session_id, .. } = event {
        let removed_explicit = runtime
            .subscription_state
            .explicit_sessions
            .remove(session_id);
        let removed_foreground = runtime
            .subscription_state
            .foreground_session_ids
            .as_mut()
            .map(|foreground| foreground.remove(session_id))
            .unwrap_or(false);
        if runtime
            .subscription_state
            .foreground_session_ids
            .as_ref()
            .is_some_and(HashSet::is_empty)
        {
            runtime.subscription_state.foreground_session_ids = None;
        }
        let removed_subscription = remove_runtime_subscription(state, runtime, *session_id).await;
        return removed_explicit || removed_foreground || removed_subscription;
    }

    if !runtime.subscription_state.active_scope {
        return true;
    }

    match event {
        WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { task, .. } => {
            let session_id = primary_session_id_for_active_task(task);
            runtime
                .subscription_state
                .active_task_sessions
                .insert(task.task.id, session_id);
            if let std::collections::hash_map::Entry::Vacant(entry) =
                runtime.subscriptions.entry(session_id)
            {
                let last_sent = state
                    .workspaces
                    .workspace_active_snapshot
                    .session_replay_cursor(workspace_id, session_id)
                    .await;
                entry.insert(SessionCursor { last_sent });
            }
        }
        WorkspaceActiveSnapshotEvent::ActiveTaskDelete { task_id, .. } => {
            remove_active_task_subscription_if_unused(state, runtime, *task_id).await;
        }
        WorkspaceActiveSnapshotEvent::TaskDelta { delta, .. }
            if matches!(delta.kind, TaskDeltaKind::Archived) =>
        {
            remove_active_task_subscription_if_unused(state, runtime, delta.task.id).await;
        }
        _ => {}
    }
    true
}

async fn remove_active_task_subscription_if_unused(
    state: &Arc<AppState>,
    runtime: &mut WorkspaceStreamRuntime,
    task_id: TaskId,
) {
    if let Some(session_id) = runtime
        .subscription_state
        .active_task_sessions
        .remove(&task_id)
    {
        let still_active = runtime
            .subscription_state
            .active_task_sessions
            .values()
            .any(|id| *id == session_id);
        if !still_active
            && !runtime
                .subscription_state
                .explicit_sessions
                .contains(&session_id)
        {
            remove_runtime_subscription(state, runtime, session_id).await;
        }
    }
}

async fn remove_runtime_subscription(
    state: &Arc<AppState>,
    runtime: &mut WorkspaceStreamRuntime,
    session_id: SessionId,
) -> bool {
    let removed = runtime.subscriptions.remove(&session_id).is_some();
    if removed {
        state.detach_session(session_id).await;
    }
    removed
}
