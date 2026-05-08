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
    let capabilities = crate::daemon::McpAuthCapabilities::provider_turn_default();
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

pub(super) fn apply_provider_mcp_command_overrides(
    provider_id: &str,
    provider_env: &mut HashMap<String, String>,
) {
    if matches!(provider_id, "fake" | "broken" | "opencode" | "kimi") {
        // These providers currently behave truthfully without the daemon MCP runtime.
        provider_env.insert("CTX_MCP_DISABLED".to_string(), "1".to_string());
    }
}

pub(super) fn strip_unused_daemon_auth_from_provider_env(
    provider_env: &mut HashMap<String, String>,
) {
    let mcp_disabled = provider_env
        .get("CTX_MCP_DISABLED")
        .and_then(|value| ctx_core::boolish::parse_boolish(value))
        .unwrap_or(false);
    if !mcp_disabled {
        return;
    }
    for key in ctx_core::env::DAEMON_AUTH_ENV_VARS {
        provider_env.remove(*key);
    }
}
