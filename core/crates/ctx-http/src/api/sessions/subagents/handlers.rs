use super::*;
use axum::extract::Extension;
use parent_session::resolve_scoped_parent_session_id;

#[path = "handlers/parent_session.rs"]
mod parent_session;

pub(crate) async fn mcp_send_input(
    State(state): State<Arc<AppState>>,
    mcp_auth: Option<Extension<crate::daemon::McpAuthContext>>,
    Path(id): Path<String>,
    Json(req): Json<SendInputReq>,
) -> Result<Json<SendInputResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = resolve_scoped_parent_session_id(&state, mcp_auth, id).await?;

    crate::daemon::sessions::subagents::send_input(state, parent_id, req)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}

pub(crate) async fn mcp_archive_agent(
    State(state): State<Arc<AppState>>,
    mcp_auth: Option<Extension<crate::daemon::McpAuthContext>>,
    Path(id): Path<String>,
    Json(req): Json<ArchiveAgentReq>,
) -> Result<Json<ArchiveAgentResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = resolve_scoped_parent_session_id(&state, mcp_auth, id).await?;

    crate::daemon::sessions::subagents::archive_agent(state, parent_id, req)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}

pub(crate) async fn mcp_list_agents(
    State(state): State<Arc<AppState>>,
    mcp_auth: Option<Extension<crate::daemon::McpAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<AgentSummary>>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = resolve_scoped_parent_session_id(&state, mcp_auth, id).await?;

    crate::daemon::sessions::subagents::list_agents(state, parent_id)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}

pub(crate) async fn mcp_get_agent(
    State(state): State<Arc<AppState>>,
    mcp_auth: Option<Extension<crate::daemon::McpAuthContext>>,
    Path(id): Path<String>,
    Json(req): Json<GetAgentReq>,
) -> Result<Json<GetAgentResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = resolve_scoped_parent_session_id(&state, mcp_auth, id).await?;

    crate::daemon::sessions::subagents::get_agent(state, parent_id, req)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}

pub(crate) async fn mcp_interrupt_agent(
    State(state): State<Arc<AppState>>,
    mcp_auth: Option<Extension<crate::daemon::McpAuthContext>>,
    Path(id): Path<String>,
    Json(req): Json<InterruptAgentReq>,
) -> Result<Json<InterruptAgentResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = resolve_scoped_parent_session_id(&state, mcp_auth, id).await?;

    crate::daemon::sessions::subagents::interrupt_agent(state, parent_id, req)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}

pub(crate) async fn mcp_wait_agent(
    State(state): State<Arc<AppState>>,
    mcp_auth: Option<Extension<crate::daemon::McpAuthContext>>,
    Path(id): Path<String>,
    Json(req): Json<WaitAgentReq>,
) -> Result<Json<WaitAgentResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = resolve_scoped_parent_session_id(&state, mcp_auth, id).await?;

    crate::daemon::sessions::subagents::wait_agent(state, parent_id, req)
        .await
        .map_err(subagent_error_response)
        .map(Json)
}
