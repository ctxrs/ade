use ctx_mcp_auth::McpAuthContext;
use ctx_observability::ops_events::{OpsEvent, OpsEvents};

pub(super) fn mcp_token_event(
    level: &str,
    event_name: &str,
    ctx: McpAuthContext,
    meta: serde_json::Value,
) -> OpsEvent {
    let mut event = OpsEvent::new(level, event_name);
    event.session_id = Some(ctx.session_id.0.to_string());
    event.worktree_id = Some(ctx.worktree_id.0.to_string());
    event.meta = Some(serde_json::json!({
        "workspace_id": ctx.workspace_id.0.to_string(),
        "capabilities": ctx.capabilities.names(),
        "detail": meta,
    }));
    event
}

pub(super) fn emit_mcp_token_event_with_ops(
    ops_events: &OpsEvents,
    level: &str,
    event_name: &str,
    ctx: McpAuthContext,
    meta: serde_json::Value,
) {
    ops_events.emit(mcp_token_event(level, event_name, ctx, meta));
}

pub(super) fn mcp_token_denied_event(
    ctx: McpAuthContext,
    method: &str,
    path: &str,
    reason: &str,
) -> OpsEvent {
    mcp_token_event(
        "warn",
        "mcp_token_denied",
        ctx,
        serde_json::json!({
            "method": method,
            "path": path,
            "reason": reason,
        }),
    )
}

pub(in crate::daemon) fn emit_mcp_token_denied_with_ops(
    ops_events: &OpsEvents,
    ctx: McpAuthContext,
    method: &str,
    path: &str,
    reason: &str,
) {
    ops_events.emit(mcp_token_denied_event(ctx, method, path, reason));
}
