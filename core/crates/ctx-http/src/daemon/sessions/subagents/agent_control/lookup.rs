use super::super::*;

pub(crate) async fn list_agents(
    state: Arc<AppState>,
    parent_id: SessionId,
) -> ApiResult<Vec<AgentSummary>> {
    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;
    let inactivity_timeout = state.provider_inactivity_timeout().await;
    let subs = store
        .list_subagent_sessions(parent.id)
        .await
        .map_err(internal_api_error)?;
    let mut agents = Vec::with_capacity(subs.len());
    for sub in subs {
        let (summary, _latest_turn) =
            build_agent_summary(&store, sub.id, &sub.title, inactivity_timeout).await?;
        agents.push(summary);
    }
    Ok(agents)
}

pub(crate) async fn get_agent(
    state: Arc<AppState>,
    parent_id: SessionId,
    req: GetAgentReq,
) -> ApiResult<GetAgentResp> {
    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;
    let inactivity_timeout = state.provider_inactivity_timeout().await;
    let child = resolve_child_agent_session(&store, &parent, &req.agent_id).await?;
    let detail = build_agent_detail(&state, &store, &parent, &child, inactivity_timeout).await?;
    Ok(GetAgentResp { agent: detail })
}
