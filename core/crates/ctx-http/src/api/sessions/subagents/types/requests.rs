use serde::Deserialize;

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

#[derive(Debug, Deserialize)]
pub(crate) struct GetAgentReq {
    pub(crate) agent_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SendInputReq {
    pub(crate) agent_id: String,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) interrupt: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArchiveAgentReq {
    pub(crate) agent_id: String,
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

#[derive(Debug, Deserialize)]
pub(crate) struct InterruptAgentReq {
    pub(crate) agent_id: String,
}
