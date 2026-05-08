use std::time::Duration;

use sha2::Digest;

use super::*;
use crate::ops_events::OpsEvent;
use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};

const PROVIDER_SESSION_MCP_AUTH_TTL: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct McpAuthCapabilities {
    pub(crate) subagents: bool,
    pub(crate) artifacts: bool,
    pub(crate) merge_queue_submit: bool,
}

impl McpAuthCapabilities {
    pub(crate) fn provider_session() -> Self {
        Self {
            subagents: true,
            artifacts: true,
            merge_queue_submit: false,
        }
    }

    pub(crate) fn provider_turn_default() -> Self {
        Self {
            subagents: true,
            artifacts: true,
            merge_queue_submit: true,
        }
    }

    pub(crate) fn names(self) -> Vec<&'static str> {
        let mut values = Vec::new();
        if self.subagents {
            values.push("subagents");
        }
        if self.artifacts {
            values.push("artifacts");
        }
        if self.merge_queue_submit {
            values.push("merge_queue_submit");
        }
        values
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct McpAuthContext {
    pub(crate) session_id: SessionId,
    pub(crate) workspace_id: WorkspaceId,
    pub(crate) worktree_id: WorktreeId,
    pub(crate) capabilities: McpAuthCapabilities,
}

impl McpAuthContext {
    pub(crate) fn provider_session(
        session_id: SessionId,
        workspace_id: WorkspaceId,
        worktree_id: WorktreeId,
        capabilities: McpAuthCapabilities,
    ) -> Self {
        Self {
            session_id,
            workspace_id,
            worktree_id,
            capabilities,
        }
    }

    pub(crate) fn allows_subagents(self, session_id: SessionId) -> bool {
        self.capabilities.subagents && self.session_id == session_id
    }

    pub(crate) fn allows_artifacts(self, session_id: SessionId) -> bool {
        self.capabilities.artifacts && self.session_id == session_id
    }

    pub(crate) fn allows_merge_queue_submit(
        self,
        session_id: SessionId,
        worktree_id: WorktreeId,
    ) -> bool {
        self.capabilities.merge_queue_submit
            && self.session_id == session_id
            && self.worktree_id == worktree_id
    }
}

fn mcp_token_hash(token: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"ctx-mcp-auth|");
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

async fn mcp_auth_registry_lock(
    state: &AppState,
) -> tokio::sync::MutexGuard<'_, HashMap<String, TimedEntry<McpAuthContext>>> {
    state.core.mcp_auth.lock().await
}

fn prune_expired_mcp_auth_entries(registry: &mut HashMap<String, TimedEntry<McpAuthContext>>) {
    registry.retain(|_, entry| entry.last_access.elapsed() <= PROVIDER_SESSION_MCP_AUTH_TTL);
}

fn revoke_matching_provider_session_mcp_tokens(
    registry: &mut HashMap<String, TimedEntry<McpAuthContext>>,
    ctx: McpAuthContext,
) -> usize {
    let before = registry.len();
    registry.retain(|_, entry| {
        entry.value.session_id != ctx.session_id
            || entry.value.workspace_id != ctx.workspace_id
            || entry.value.worktree_id != ctx.worktree_id
    });
    before.saturating_sub(registry.len())
}

fn emit_mcp_token_event(
    state: &AppState,
    level: &str,
    event_name: &str,
    ctx: McpAuthContext,
    meta: serde_json::Value,
) {
    let mut event = OpsEvent::new(level, event_name);
    event.session_id = Some(ctx.session_id.0.to_string());
    event.worktree_id = Some(ctx.worktree_id.0.to_string());
    event.meta = Some(serde_json::json!({
        "workspace_id": ctx.workspace_id.0.to_string(),
        "capabilities": ctx.capabilities.names(),
        "detail": meta,
    }));
    state.telemetry.ops_events.emit(event);
}

pub(crate) fn emit_mcp_token_denied(
    state: &AppState,
    ctx: McpAuthContext,
    method: &str,
    path: &str,
    reason: &str,
) {
    emit_mcp_token_event(
        state,
        "warn",
        "mcp_token_denied",
        ctx,
        serde_json::json!({
            "method": method,
            "path": path,
            "reason": reason,
        }),
    );
}

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
    let mut registry = mcp_auth_registry_lock(state).await;
    prune_expired_mcp_auth_entries(&mut registry);
    let replaced_count = revoke_matching_provider_session_mcp_tokens(&mut registry, ctx);
    registry.insert(token_hash, TimedEntry::new(ctx));
    drop(registry);
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
    let mut registry = mcp_auth_registry_lock(state).await;
    prune_expired_mcp_auth_entries(&mut registry);
    let revoked = registry.remove(&token_hash).map(|entry| entry.value);
    drop(registry);
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
    let mut registry = mcp_auth_registry_lock(state).await;
    prune_expired_mcp_auth_entries(&mut registry);
    let entry = registry.get_mut(&token_hash)?;
    entry.touch();
    Some(entry.value)
}
