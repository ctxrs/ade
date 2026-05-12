use super::*;

pub(super) async fn issue_mcp_token_if_enabled(
    state: &Arc<AppState>,
    session: &Session,
    provider_env: &mut HashMap<String, String>,
    mcp_disabled: bool,
) -> Option<String> {
    if mcp_disabled {
        return None;
    }
    let capabilities = ctx_mcp_auth::McpAuthCapabilities::provider_turn_default();
    let token = crate::daemon::issue_provider_session_mcp_token_with_capabilities(
        state.as_ref(),
        session.id,
        session.workspace_id,
        session.worktree_id,
        capabilities,
    )
    .await;
    provider_env.insert("CTX_MCP_TOKEN".to_string(), token.clone());
    Some(token)
}
