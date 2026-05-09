use axum::body::Body;
use axum::http::{Method, Request};
use ctx_core::ids::SessionId;

#[derive(Clone, Copy)]
pub(in crate::api::auth) enum ScopedMcpRoute {
    SessionSubagents { session_id: SessionId },
    SessionArtifacts { session_id: SessionId },
    MergeQueueSubmit,
}

pub(in crate::api::auth) fn scoped_mcp_route(req: &Request<Body>) -> Option<ScopedMcpRoute> {
    let path = req.uri().path();
    if req.method() == Method::POST && path == "/api/merge-queue/entries" {
        return Some(ScopedMcpRoute::MergeQueueSubmit);
    }
    if let Some(session_id) = parse_scoped_mcp_session_id(
        path,
        "/api/mcp/sessions/",
        &[
            "spawn_agent",
            "send_input",
            "archive_agent",
            "interrupt_agent",
            "list_agents",
            "get_agent",
            "wait_agent",
        ],
    ) {
        return Some(ScopedMcpRoute::SessionSubagents { session_id });
    }
    if req.method() == Method::POST {
        if let Some(session_id) =
            parse_scoped_mcp_session_id(path, "/api/sessions/", &["artifacts"])
        {
            return Some(ScopedMcpRoute::SessionArtifacts { session_id });
        }
    }
    None
}

fn parse_scoped_mcp_session_id(
    path: &str,
    prefix: &str,
    allowed_suffixes: &[&str],
) -> Option<SessionId> {
    let remainder = path.strip_prefix(prefix)?;
    let (raw_session_id, suffix) = remainder.split_once('/')?;
    if !allowed_suffixes.contains(&suffix) {
        return None;
    }
    let parsed = uuid::Uuid::parse_str(raw_session_id).ok()?;
    Some(SessionId(parsed))
}
