use super::*;

pub(super) async fn collect_turns_by_statuses(
    state: &Arc<DaemonState>,
    statuses: &[SessionTurnStatus],
) -> Result<(usize, Vec<(WorkspaceId, SessionTurn)>)> {
    let workspaces = state.global_store().list_workspaces().await?;
    let workspace_count = workspaces.len();
    let mut matching_turns = Vec::new();
    for workspace in workspaces {
        let store = state
            .core
            .stores
            .workspace_transient(workspace.id)
            .await
            .with_context(|| {
                format!(
                    "failed to open workspace {} while collecting turns by status",
                    workspace.id.0
                )
            })?;
        let turns = store
            .list_session_turns_by_statuses(statuses)
            .await
            .with_context(|| {
                format!(
                    "failed to list workspace {} turns by status",
                    workspace.id.0
                )
            });
        store.close().await;
        let mut turns = turns?;
        matching_turns.extend(turns.drain(..).map(|turn| (workspace.id, turn)));
    }
    Ok((workspace_count, matching_turns))
}

pub(super) async fn session_execution_environment(
    state: &Arc<DaemonState>,
    cache: &mut HashMap<ctx_core::ids::SessionId, ExecutionEnvironment>,
    session_id: ctx_core::ids::SessionId,
) -> Result<ExecutionEnvironment> {
    if let Some(environment) = cache.get(&session_id).copied() {
        return Ok(environment);
    }
    let store = state.store_for_session(session_id).await?;
    let session = store
        .get_session(session_id)
        .await?
        .with_context(|| format!("missing session {} for sandbox activity", session_id.0))?;
    let environment = session.execution_environment;
    cache.insert(session_id, environment);
    Ok(environment)
}
