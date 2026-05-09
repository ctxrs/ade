use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};

use crate::daemon::AppState;

mod events;
mod registry;
mod types;

pub(crate) use events::emit_mcp_token_denied;
use events::emit_mcp_token_event;
use registry::{
    insert_provider_session_mcp_token, mcp_token_hash, remove_provider_session_mcp_token,
    verify_provider_session_mcp_token,
};
pub(crate) use types::{McpAuthCapabilities, McpAuthContext};

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
    let token = format!("ctxmcp_{}", uuid::Uuid::new_v4().simple());
    let token_hash = mcp_token_hash(&token);
    let ctx = McpAuthContext::provider_session(session_id, workspace_id, worktree_id, capabilities);
    let replaced_count = insert_provider_session_mcp_token(state, token_hash, ctx).await;
    if replaced_count > 0 {
        emit_mcp_token_event(
            state,
            "info",
            "mcp_token_revoked",
            ctx,
            serde_json::json!({ "reason": "replaced", "count": replaced_count }),
        );
    }
    emit_mcp_token_event(
        state,
        "info",
        "mcp_token_issued",
        ctx,
        serde_json::json!({ "reason": "provider_session" }),
    );
    token
}

pub(crate) async fn revoke_provider_session_mcp_token(state: &AppState, token: &str) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    let token_hash = mcp_token_hash(token);
    let revoked = remove_provider_session_mcp_token(state, &token_hash).await;
    if let Some(ctx) = revoked {
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
    let token_hash = mcp_token_hash(token);
    verify_provider_session_mcp_token(state, &token_hash).await
}
