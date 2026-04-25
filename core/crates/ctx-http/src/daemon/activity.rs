use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct ActiveTurnRecord {
    pub workspace_id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub turn_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonTurnActivitySummary {
    pub idle: bool,
    pub active_turn_count: usize,
    pub queued_turn_count: usize,
    pub running_turn_count: usize,
    pub scanned_workspace_count: usize,
    pub turns: Vec<ActiveTurnRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_drain: Option<UpdateDrainState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonSandboxWorkActivitySummary {
    pub active: bool,
    pub active_sandbox_turn_count: usize,
    pub queued_sandbox_turn_count: usize,
    pub running_sandbox_turn_count: usize,
    pub running_container_backed_terminal: bool,
    pub running_workspace_container_count: usize,
    pub runtime_operation_count: usize,
    pub prewarm_artifact_operation_count: usize,
    pub scanned_workspace_count: usize,
    pub turns: Vec<ActiveTurnRecord>,
}

fn turn_status_name(status: &SessionTurnStatus) -> &'static str {
    match status {
        SessionTurnStatus::Queued => "queued",
        SessionTurnStatus::Starting => "starting",
        SessionTurnStatus::Running => "running",
        SessionTurnStatus::Completed => "completed",
        SessionTurnStatus::Failed => "failed",
        SessionTurnStatus::Interrupted => "interrupted",
    }
}

async fn collect_turns_by_statuses(
    state: &Arc<AppState>,
    statuses: &[SessionTurnStatus],
) -> Result<(usize, Vec<(WorkspaceId, SessionTurn)>)> {
    let workspaces = state.global_store().list_workspaces().await?;
    let workspace_count = workspaces.len();
    let mut matching_turns = Vec::new();
    for workspace in workspaces {
        let store = state.core.stores.workspace_transient(workspace.id).await?;
        let mut turns = store.list_session_turns_by_statuses(statuses).await?;
        store.close().await;
        matching_turns.extend(turns.drain(..).map(|turn| (workspace.id, turn)));
    }
    Ok((workspace_count, matching_turns))
}

async fn session_execution_environment(
    state: &Arc<AppState>,
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

pub async fn daemon_turn_activity_summary(
    state: &Arc<AppState>,
) -> Result<DaemonTurnActivitySummary> {
    let (workspace_count, turns) = collect_turns_by_statuses(
        state,
        &[
            SessionTurnStatus::Queued,
            SessionTurnStatus::Starting,
            SessionTurnStatus::Running,
        ],
    )
    .await?;
    let queued_turn_count = turns
        .iter()
        .filter(|(_, turn)| matches!(&turn.status, SessionTurnStatus::Queued))
        .count();
    let running_turn_count = turns
        .iter()
        .filter(|(_, turn)| {
            matches!(
                &turn.status,
                SessionTurnStatus::Starting | SessionTurnStatus::Running
            )
        })
        .count();
    let records = turns
        .into_iter()
        .map(|(workspace_id, turn)| ActiveTurnRecord {
            workspace_id: workspace_id.0.to_string(),
            session_id: turn.session_id.0.to_string(),
            run_id: turn.run_id.map(|run_id| run_id.0.to_string()),
            turn_id: turn.turn_id.0.to_string(),
            status: turn_status_name(&turn.status).to_string(),
        })
        .collect::<Vec<_>>();
    let active_turn_count = queued_turn_count + running_turn_count;
    Ok(DaemonTurnActivitySummary {
        idle: active_turn_count == 0,
        active_turn_count,
        queued_turn_count,
        running_turn_count,
        scanned_workspace_count: workspace_count,
        turns: records,
        update_drain: state.update_drain_snapshot().await,
    })
}

pub async fn daemon_sandbox_work_activity_summary(
    state: &Arc<AppState>,
) -> Result<DaemonSandboxWorkActivitySummary> {
    let (workspace_count, turns) = collect_turns_by_statuses(
        state,
        &[
            SessionTurnStatus::Queued,
            SessionTurnStatus::Starting,
            SessionTurnStatus::Running,
        ],
    )
    .await?;
    let mut session_env_cache = HashMap::new();
    let mut records = Vec::new();
    let mut queued_sandbox_turn_count = 0usize;
    let mut running_sandbox_turn_count = 0usize;

    for (workspace_id, turn) in turns {
        if !matches!(
            session_execution_environment(state, &mut session_env_cache, turn.session_id).await?,
            ExecutionEnvironment::Sandbox
        ) {
            continue;
        }
        if matches!(turn.status, SessionTurnStatus::Queued) {
            queued_sandbox_turn_count += 1;
        }
        if matches!(
            turn.status,
            SessionTurnStatus::Starting | SessionTurnStatus::Running
        ) {
            running_sandbox_turn_count += 1;
        }
        records.push(ActiveTurnRecord {
            workspace_id: workspace_id.0.to_string(),
            session_id: turn.session_id.0.to_string(),
            run_id: turn.run_id.map(|run_id| run_id.0.to_string()),
            turn_id: turn.turn_id.0.to_string(),
            status: turn_status_name(&turn.status).to_string(),
        });
    }

    let running_container_backed_terminal = state
        .transport
        .terminals
        .has_running_container_backed()
        .await;
    let running_workspace_container_count = state
        .execution
        .harness
        .running_workspace_container_count()
        .await?;
    let runtime_operation_count = state.execution.harness.runtime_operation_count();
    let prewarm_artifact_operation_count =
        state.execution.harness.prewarm_artifact_operation_count();
    let active_sandbox_turn_count = queued_sandbox_turn_count + running_sandbox_turn_count;
    Ok(DaemonSandboxWorkActivitySummary {
        active: active_sandbox_turn_count > 0
            || running_container_backed_terminal
            || running_workspace_container_count > 0
            || runtime_operation_count > 0
            || prewarm_artifact_operation_count > 0,
        active_sandbox_turn_count,
        queued_sandbox_turn_count,
        running_sandbox_turn_count,
        running_container_backed_terminal,
        running_workspace_container_count,
        runtime_operation_count,
        prewarm_artifact_operation_count,
        scanned_workspace_count: workspace_count,
        turns: records,
    })
}

pub(super) async fn reconcile_running_turns(state: &Arc<AppState>) -> Result<()> {
    let (_, running_turns) = collect_turns_by_statuses(
        state,
        &[SessionTurnStatus::Starting, SessionTurnStatus::Running],
    )
    .await?;

    for (_, turn) in running_turns {
        if let Err(err) = reconcile_turn_terminal_state(
            state,
            turn.session_id,
            turn.run_id,
            turn.turn_id,
            "daemon_restart",
        )
        .await
        {
            tracing::warn!(
                session_id = %turn.session_id.0,
                turn_id = %turn.turn_id.0,
                err = %err,
                "failed to reconcile running turn after daemon restart"
            );
        }
    }

    Ok(())
}
