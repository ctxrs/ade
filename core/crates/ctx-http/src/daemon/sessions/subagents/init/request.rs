use std::sync::Arc;

use crate::daemon::sessions::subagents::errors::{
    api_error, internal_api_error, ApiResult, SubagentErrorKind,
};
use crate::daemon::AppState;
use ctx_session_service::subagents::{
    build_subagent_request_json, normalize_subagent_labels, parse_subagent_worktree,
    resolve_max_subagents_per_call, SubagentRequestAgent, SubagentWorktreeSelection,
};

use super::super::{AgentInitItem, AgentInitReq};

pub(super) struct PreparedSubagentInitRequest {
    pub(super) agents: Vec<AgentInitItem>,
    pub(super) labels: Vec<String>,
    pub(super) request_json: serde_json::Value,
    pub(super) tool_call_id: Option<String>,
    pub(super) worktree_selection: SubagentWorktreeSelection,
}

pub(super) async fn prepare_subagent_init_request(
    state: &Arc<AppState>,
    req: &AgentInitReq,
) -> ApiResult<PreparedSubagentInitRequest> {
    if req.agents.is_empty() {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "agents is required",
        ));
    }

    let settings = ctx_settings_service::load_settings(state.global_store())
        .await
        .map_err(internal_api_error)?;
    let max_subagents =
        resolve_max_subagents_per_call(settings.subagents.as_ref().and_then(|s| s.max_per_call));
    if req.agents.len() > max_subagents {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            format!("max {max_subagents} subagents per call"),
        ));
    }
    if req
        .response_mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "response_mode is not supported; use wait_agent to await",
        ));
    }
    let worktree_selection = parse_subagent_worktree(req.worktree.as_deref())
        .map_err(|error| api_error(SubagentErrorKind::BadRequest, error))?;

    let agents = req.agents.clone();
    let request_agents = build_subagent_request_agents(&agents);
    let labels = normalize_subagent_labels(&request_agents)
        .map_err(|error| api_error(SubagentErrorKind::BadRequest, error))?;
    let request_json = build_subagent_request_json(&request_agents);
    Ok(PreparedSubagentInitRequest {
        agents,
        labels,
        request_json,
        tool_call_id: req.tool_call_id.clone(),
        worktree_selection,
    })
}

pub(super) fn build_subagent_request_agents(
    agents: &[AgentInitItem],
) -> Vec<SubagentRequestAgent<'_>> {
    agents
        .iter()
        .map(|agent| SubagentRequestAgent {
            prompt: &agent.prompt,
            label: agent.label.as_deref(),
            harness: agent.harness.as_deref(),
            model: agent.model.as_deref(),
            reasoning_effort: agent.reasoning_effort.as_deref(),
        })
        .collect::<Vec<_>>()
}
