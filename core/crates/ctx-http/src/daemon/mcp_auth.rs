use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};

use crate::daemon::AppState;

mod events;

use ctx_mcp_auth::{McpAuthCapabilities, McpAuthContext};
pub(crate) use events::emit_mcp_token_denied;
use events::emit_mcp_token_event;

pub async fn issue_provider_session_mcp_token(
    state: &AppState,
    session_id: SessionId,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
) -> String {
    issue_provider_session_mcp_token_with_capabilities(
        state,
        session_id,
        workspace_id,
        worktree_id,
        McpAuthCapabilities::provider_session(),
    )
    .await
}

pub(crate) async fn issue_provider_session_mcp_token_with_capabilities(
    state: &AppState,
    session_id: SessionId,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    capabilities: McpAuthCapabilities,
) -> String {
    let issued = state
        .core
        .mcp_auth
        .issue_provider_session_token_with_capabilities(
            session_id,
            workspace_id,
            worktree_id,
            capabilities,
        )
        .await;
    if issued.replaced_count > 0 {
        emit_mcp_token_event(
            state,
            "info",
            "mcp_token_revoked",
            issued.context,
            serde_json::json!({ "reason": "replaced", "count": issued.replaced_count }),
        );
    }
    emit_mcp_token_event(
        state,
        "info",
        "mcp_token_issued",
        issued.context,
        serde_json::json!({ "reason": "provider_session" }),
    );
    issued.token
}

pub(crate) async fn revoke_provider_session_mcp_token(state: &AppState, token: &str) -> bool {
    if let Some(ctx) = state
        .core
        .mcp_auth
        .revoke_provider_session_token(token)
        .await
    {
        emit_mcp_token_event(
            state,
            "info",
            "mcp_token_revoked",
            ctx,
            serde_json::json!({ "reason": "explicit" }),
        );
        return true;
    }
    false
}

pub(crate) async fn verify_mcp_auth_token(state: &AppState, token: &str) -> Option<McpAuthContext> {
    state.core.mcp_auth.verify_token(token).await
}
