use super::*;

pub(super) async fn emit_session_started_observability(
    state: &Arc<AppState>,
    session: &Session,
    task: &Task,
) {
    let worktree = match state.store_for_session(session.id).await {
        Ok(store) => store.get_worktree(session.worktree_id).await.ok().flatten(),
        Err(_) => None,
    };
    let session_root_kind = session_root_kind_for_worktree(worktree.as_ref()).to_string();
    state
        .telemetry
        .telemetry
        .emit(TelemetryEvent::session_started(
            session.provider_id.clone(),
            compose_model_id(&session.model_id, session.reasoning_effort.as_deref()),
            Some(session.execution_environment.as_str().to_string()),
            Some(session_root_kind.clone()),
        ))
        .await;
    let mut ops_event = OpsEvent::new("info", "session_started");
    ops_event.session_id = Some(session.id.0.to_string());
    ops_event.worktree_id = Some(session.worktree_id.0.to_string());
    ops_event.provider_id = Some(session.provider_id.clone());
    ops_event.meta = Some(serde_json::json!({
        "model_id": compose_model_id(&session.model_id, session.reasoning_effort.as_deref()),
        "reasoning_effort": session.reasoning_effort.clone(),
        "execution_environment": session.execution_environment.as_str(),
        "session_root_kind": session_root_kind,
        "parent_session_id": session.parent_session_id.map(|id| id.0.to_string()),
        "relationship": session.relationship.clone(),
    }));
    state.telemetry.ops_events.emit(ops_event);
    if let Err(e) = state.emit_workspace_task_upsert(task.id).await {
        tracing::warn!(task_id = %task.id.0, "workspace active snapshot refresh failed: {e:?}");
    }
}
