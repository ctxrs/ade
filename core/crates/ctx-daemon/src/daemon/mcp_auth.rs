use anyhow::Error;
use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};
use ctx_observability::ops_events::OpsEvents;

mod events;

use ctx_mcp_auth::McpAuthCapabilities;
pub(in crate::daemon) use events::emit_mcp_token_denied_with_ops;
use events::emit_mcp_token_event_with_ops;

pub async fn issue_provider_session_mcp_token_with_capabilities_parts(
    mcp_auth: &ctx_mcp_auth::McpAuthRegistry,
    ops_events: &OpsEvents,
    session_id: SessionId,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    capabilities: McpAuthCapabilities,
) -> String {
    let issued = mcp_auth
        .issue_provider_session_token_with_capabilities(
            session_id,
            workspace_id,
            worktree_id,
            capabilities,
        )
        .await;
    if issued.replaced_count > 0 {
        emit_mcp_token_event_with_ops(
            ops_events,
            "info",
            "mcp_token_revoked",
            issued.context,
            serde_json::json!({ "reason": "replaced", "count": issued.replaced_count }),
        );
    }
    emit_mcp_token_event_with_ops(
        ops_events,
        "info",
        "mcp_token_issued",
        issued.context,
        serde_json::json!({ "reason": "provider_session" }),
    );
    issued.token
}

pub async fn revoke_provider_session_mcp_token_parts(
    mcp_auth: &ctx_mcp_auth::McpAuthRegistry,
    ops_events: &OpsEvents,
    token: &str,
) -> bool {
    if let Some(ctx) = mcp_auth.revoke_provider_session_token(token).await {
        emit_mcp_token_event_with_ops(
            ops_events,
            "info",
            "mcp_token_revoked",
            ctx,
            serde_json::json!({ "reason": "explicit" }),
        );
        return true;
    }
    false
}

#[derive(Debug)]
pub enum ScopedMcpSessionAccessError {
    Unauthorized(&'static str),
    SessionNotFound,
    StoreUnavailable(Error),
}

#[cfg(test)]
mod tests;
