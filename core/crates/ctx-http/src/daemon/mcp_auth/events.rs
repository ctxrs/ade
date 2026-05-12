use ctx_mcp_auth::McpAuthContext;
use ctx_observability::ops_events::OpsEvent;

use crate::daemon::AppState;

pub(super) fn emit_mcp_token_event(
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
