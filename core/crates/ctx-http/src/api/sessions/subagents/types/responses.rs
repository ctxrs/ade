use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub(crate) struct ContextWindowSummary {
    pub(crate) total: u64,
    pub(crate) used: u64,
    pub(crate) remaining: u64,
    pub(crate) utilization: f64,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct AgentSummary {
    pub(crate) agent_id: String,
    pub(crate) task_label: String,
    pub(crate) state: String,
    pub(crate) health: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) latest_result_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_progress_at: Option<String>,
    pub(crate) last_event_seq: i64,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct AgentResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) run_id: Option<String>,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) context_window: Option<ContextWindowSummary>,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct AgentDetail {
    pub(crate) agent: AgentSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) latest_result: Option<AgentResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worktree_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SpawnAgentResp {
    pub(crate) agent: AgentDetail,
}

#[derive(Debug, Serialize)]
pub(crate) struct GetAgentResp {
    pub(crate) agent: AgentDetail,
}

#[derive(Debug, Serialize)]
pub(crate) struct SendInputResp {
    pub(crate) agent: AgentDetail,
    pub(crate) queued_run_id: String,
    pub(crate) delivery: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ArchiveAgentResp {
    pub(crate) agent_id: String,
    pub(crate) task_label: String,
    pub(crate) archived: bool,
    pub(crate) cleanup_failed: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct WaitAgentResp {
    pub(crate) wait_status: String,
    pub(crate) mode: String,
    pub(crate) until: String,
    pub(crate) results: Vec<AgentDetail>,
}

#[derive(Debug, Serialize)]
pub(crate) struct InterruptAgentResp {
    pub(crate) agent: AgentDetail,
}
