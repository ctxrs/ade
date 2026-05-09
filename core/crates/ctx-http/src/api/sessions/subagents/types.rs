use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct AgentInitReq {
    #[serde(default)]
    pub(crate) tool_call_id: Option<String>,
    #[serde(default)]
    pub(crate) response_mode: Option<String>,
    #[serde(default)]
    pub(crate) worktree: Option<String>,
    pub(crate) agents: Vec<AgentInitItem>,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct AgentInitItem {
    pub(crate) prompt: String,
    #[serde(default)]
    pub(crate) label: Option<String>,
    #[serde(default)]
    pub(crate) harness: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) reasoning_effort: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct ContextWindowSummary {
    pub(crate) total: u64,
    pub(crate) used: u64,
    pub(crate) remaining: u64,
    pub(crate) utilization: f64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SpawnAgentReq {
    #[serde(default)]
    pub(crate) tool_call_id: Option<String>,
    #[serde(default)]
    pub(crate) worktree: Option<String>,
    pub(crate) task_label: String,
    pub(crate) prompt: String,
    #[serde(default)]
    pub(crate) harness: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) reasoning_effort: Option<String>,
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

#[derive(Debug, Deserialize)]
pub(crate) struct GetAgentReq {
    pub(crate) agent_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct GetAgentResp {
    pub(crate) agent: AgentDetail,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SendInputReq {
    pub(crate) agent_id: String,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) interrupt: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SendInputResp {
    pub(crate) agent: AgentDetail,
    pub(crate) queued_run_id: String,
    pub(crate) delivery: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArchiveAgentReq {
    pub(crate) agent_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ArchiveAgentResp {
    pub(crate) agent_id: String,
    pub(crate) task_label: String,
    pub(crate) archived: bool,
    pub(crate) cleanup_failed: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WaitAgentReq {
    #[serde(default)]
    pub(crate) agent_id: Option<String>,
    #[serde(default)]
    pub(crate) agent_ids: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) mode: Option<String>,
    #[serde(default)]
    pub(crate) until: Option<String>,
    #[serde(default)]
    pub(crate) since_seq: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WaitAgentResp {
    pub(crate) wait_status: String,
    pub(crate) mode: String,
    pub(crate) until: String,
    pub(crate) results: Vec<AgentDetail>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InterruptAgentReq {
    pub(crate) agent_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct InterruptAgentResp {
    pub(crate) agent: AgentDetail,
}
