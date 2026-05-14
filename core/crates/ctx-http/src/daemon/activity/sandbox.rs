use super::collect::{collect_turns_by_statuses, session_execution_environment};
use super::types::{active_turn_record, DaemonSandboxWorkActivitySummary};
use super::*;

pub async fn daemon_sandbox_work_activity_summary(
    state: &Arc<DaemonState>,
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
        records.push(active_turn_record(workspace_id, turn));
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
