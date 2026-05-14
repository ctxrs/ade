use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AgentInitReq {
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub response_mode: Option<String>,
    #[serde(default)]
    pub worktree: Option<String>,
    pub agents: Vec<AgentInitItem>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AgentInitItem {
    pub prompt: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SpawnAgentReq {
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub worktree: Option<String>,
    pub task_label: String,
    pub prompt: String,
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GetAgentReq {
    pub agent_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SendInputReq {
    pub agent_id: String,
    pub message: String,
    #[serde(default)]
    pub interrupt: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ArchiveAgentReq {
    pub agent_id: String,
}

#[derive(Debug, Deserialize)]
pub struct WaitAgentReq {
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_ids: Option<Vec<String>>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub until: Option<String>,
    #[serde(default)]
    pub since_seq: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct InterruptAgentReq {
    pub agent_id: String,
}
