use super::super::*;

pub async fn spawn_agent(
    state: Arc<DaemonState>,
    parent_id: SessionId,
    req: SpawnAgentReq,
) -> ApiResult<SpawnAgentResp> {
    let task_label = req.task_label.trim().to_string();
    if task_label.is_empty() {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "task_label is required",
        ));
    }
    let prompt = req.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "prompt is required",
        ));
    }

    let spawned_children = init_subagents(
        state,
        parent_id,
        AgentInitReq {
            tool_call_id: req.tool_call_id,
            response_mode: None,
            worktree: req.worktree,
            agents: vec![AgentInitItem {
                prompt,
                label: Some(task_label.clone()),
                harness: req.harness,
                model: req.model,
                reasoning_effort: req.reasoning_effort,
            }],
        },
    )
    .await?;
    let spawned = spawned_children
        .into_iter()
        .next()
        .ok_or_else(|| api_error(SubagentErrorKind::NotFound, "spawned agent not found"))?;
    Ok(SpawnAgentResp {
        agent: build_spawned_agent_detail(&spawned),
    })
}
