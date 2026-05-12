use super::*;
use ctx_update_service::UpdateDrainState;

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

pub(super) fn active_turn_record(workspace_id: WorkspaceId, turn: SessionTurn) -> ActiveTurnRecord {
    ActiveTurnRecord {
        workspace_id: workspace_id.0.to_string(),
        session_id: turn.session_id.0.to_string(),
        run_id: turn.run_id.map(|run_id| run_id.0.to_string()),
        turn_id: turn.turn_id.0.to_string(),
        status: turn_status_name(&turn.status).to_string(),
    }
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
