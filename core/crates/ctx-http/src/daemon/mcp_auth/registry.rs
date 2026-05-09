use std::collections::HashMap;
use std::time::Duration;

use sha2::Digest;

use crate::daemon::{AppState, TimedEntry};

use super::McpAuthContext;

const PROVIDER_SESSION_MCP_AUTH_TTL: Duration = Duration::from_secs(12 * 60 * 60);

pub(super) fn mcp_token_hash(token: &str) -> String {
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

pub(super) async fn insert_provider_session_mcp_token(
    state: &AppState,
    token_hash: String,
    ctx: McpAuthContext,
) -> usize {
    let mut registry = mcp_auth_registry_lock(state).await;
    prune_expired_mcp_auth_entries(&mut registry);
    let replaced_count = revoke_matching_provider_session_mcp_tokens(&mut registry, ctx);
    registry.insert(token_hash, TimedEntry::new(ctx));
    replaced_count
}

pub(super) async fn remove_provider_session_mcp_token(
    state: &AppState,
    token_hash: &str,
) -> Option<McpAuthContext> {
    let mut registry = mcp_auth_registry_lock(state).await;
    prune_expired_mcp_auth_entries(&mut registry);
    registry.remove(token_hash).map(|entry| entry.value)
}

pub(super) async fn verify_provider_session_mcp_token(
    state: &AppState,
    token_hash: &str,
) -> Option<McpAuthContext> {
    let mut registry = mcp_auth_registry_lock(state).await;
    prune_expired_mcp_auth_entries(&mut registry);
    let entry = registry.get_mut(token_hash)?;
    entry.touch();
    Some(entry.value)
}
