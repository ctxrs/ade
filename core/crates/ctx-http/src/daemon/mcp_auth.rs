use std::time::Duration;

use sha2::Digest;

use super::*;
use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};

const PROVIDER_SESSION_MCP_AUTH_TTL: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct McpAuthCapabilities {
    pub(crate) subagents: bool,
    pub(crate) artifacts: bool,
    pub(crate) merge_queue_submit: bool,
}

impl McpAuthCapabilities {
    fn provider_session() -> Self {
        Self {
            subagents: true,
            artifacts: true,
            merge_queue_submit: true,
        }
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
    ) -> Self {
        Self {
            session_id,
            workspace_id,
            worktree_id,
            capabilities: McpAuthCapabilities::provider_session(),
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
) {
    registry.retain(|_, entry| {
        entry.value.session_id != ctx.session_id
            || entry.value.workspace_id != ctx.workspace_id
            || entry.value.worktree_id != ctx.worktree_id
    });
}

pub async fn issue_provider_session_mcp_token(
    state: &AppState,
    session_id: SessionId,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
) -> String {
    let token = format!("ctxmcp_{}", uuid::Uuid::new_v4().simple());
    let token_hash = mcp_token_hash(&token);
    let ctx = McpAuthContext::provider_session(session_id, workspace_id, worktree_id);
    let mut registry = mcp_auth_registry_lock(state).await;
    prune_expired_mcp_auth_entries(&mut registry);
    revoke_matching_provider_session_mcp_tokens(&mut registry, ctx);
    registry.insert(token_hash, TimedEntry::new(ctx));
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
    registry.remove(&token_hash).is_some()
}

pub(crate) async fn verify_mcp_auth_token(state: &AppState, token: &str) -> Option<McpAuthContext> {
    let token_hash = mcp_token_hash(token);
    let mut registry = mcp_auth_registry_lock(state).await;
    prune_expired_mcp_auth_entries(&mut registry);
    let entry = registry.get_mut(&token_hash)?;
    entry.touch();
    Some(entry.value)
}
