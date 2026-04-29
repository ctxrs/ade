use axum::body::Body;
use axum::http::{Method, Request};
use sha2::Digest;
use url::form_urlencoded;

use ctx_core::ids::SessionId;

#[derive(Clone, Copy)]
pub(super) enum ScopedMcpRoute {
    SessionSubagents { session_id: SessionId },
    SessionArtifacts { session_id: SessionId },
    MergeQueueSubmit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserStreamAuthScope {
    WorkspaceActiveSnapshot { workspace_id: String },
    WorkspaceStream { workspace_id: String },
    ExecutionLaunch { job_id: String },
    DictationLivekit,
    ProviderInstall { install_id: String },
}

impl BrowserStreamAuthScope {
    fn serialize(&self) -> String {
        match self {
            Self::WorkspaceActiveSnapshot { workspace_id } => {
                format!("workspace_active_snapshot:{workspace_id}")
            }
            Self::WorkspaceStream { workspace_id } => format!("workspace_stream:{workspace_id}"),
            Self::ExecutionLaunch { job_id } => format!("execution_launch:{job_id}"),
            Self::DictationLivekit => "dictation_livekit".to_string(),
            Self::ProviderInstall { install_id } => format!("provider_install:{install_id}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowserCapabilityAuthScope {
    Blob {
        blob_id: String,
    },
    SessionArtifact {
        session_id: String,
        artifact_id: String,
    },
}

impl BrowserCapabilityAuthScope {
    fn serialize(&self) -> String {
        match self {
            Self::Blob { blob_id } => format!("blob:{blob_id}"),
            Self::SessionArtifact {
                session_id,
                artifact_id,
            } => format!("session_artifact:{session_id}:{artifact_id}"),
        }
    }
}

pub(super) fn is_websocket_upgrade(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(axum::http::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
        || headers.contains_key("sec-websocket-key")
}

fn query_param(req: &Request<Body>, key: &str) -> Option<String> {
    let query = req.uri().query()?;
    form_urlencoded::parse(query.as_bytes())
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.into_owned())
}

fn workspace_stream_scope(path: &str) -> Option<BrowserStreamAuthScope> {
    let remainder = path.strip_prefix("/api/workspaces/")?;
    let (workspace_id, suffix) = remainder.split_once('/')?;
    let workspace_id = workspace_id.trim();
    if workspace_id.is_empty() {
        return None;
    }
    match suffix {
        "active_snapshot/stream" => Some(BrowserStreamAuthScope::WorkspaceActiveSnapshot {
            workspace_id: workspace_id.to_string(),
        }),
        "stream" => Some(BrowserStreamAuthScope::WorkspaceStream {
            workspace_id: workspace_id.to_string(),
        }),
        _ => None,
    }
}

fn provider_install_stream_scope(path: &str) -> Option<BrowserStreamAuthScope> {
    let remainder = path.strip_prefix("/api/providers/install/")?;
    let (install_id, suffix) = remainder.split_once('/')?;
    let install_id = install_id.trim();
    if install_id.is_empty() || suffix != "stream" {
        return None;
    }
    Some(BrowserStreamAuthScope::ProviderInstall {
        install_id: install_id.to_string(),
    })
}

fn browser_capability_scope(req: &Request<Body>) -> Option<BrowserCapabilityAuthScope> {
    let path = req.uri().path();
    if let Some(blob_id) = path.strip_prefix("/api/blobs/") {
        let blob_id = blob_id.trim();
        if blob_id.is_empty() || blob_id.contains('/') {
            return None;
        }
        return Some(BrowserCapabilityAuthScope::Blob {
            blob_id: blob_id.to_string(),
        });
    }

    let remainder = path.strip_prefix("/api/sessions/")?;
    let (session_id, suffix) = remainder.split_once('/')?;
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return None;
    }
    let artifact_suffix = suffix.strip_prefix("artifacts/")?;
    let artifact_id = artifact_suffix.trim();
    if artifact_id.is_empty() || artifact_id.contains('/') {
        return None;
    }
    Some(BrowserCapabilityAuthScope::SessionArtifact {
        session_id: session_id.to_string(),
        artifact_id: artifact_id.to_string(),
    })
}

fn browser_stream_scope(req: &Request<Body>) -> Option<BrowserStreamAuthScope> {
    let path = req.uri().path();
    if let Some(scope) = workspace_stream_scope(path) {
        return Some(scope);
    }
    if let Some(scope) = provider_install_stream_scope(path) {
        return Some(scope);
    }
    match path {
        "/api/execution/launch/stream" => {
            let job_id = query_param(req, "job_id")?;
            let job_id = job_id.trim();
            if job_id.is_empty() {
                return None;
            }
            Some(BrowserStreamAuthScope::ExecutionLaunch {
                job_id: job_id.to_string(),
            })
        }
        "/api/dictation/livekit/stream" => Some(BrowserStreamAuthScope::DictationLivekit),
        _ => None,
    }
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

pub(super) fn scoped_mcp_route(req: &Request<Body>) -> Option<ScopedMcpRoute> {
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
    if let Some(session_id) = parse_scoped_mcp_session_id(path, "/api/sessions/", &["artifacts"]) {
        return Some(ScopedMcpRoute::SessionArtifacts { session_id });
    }
    None
}

pub(crate) fn derive_browser_stream_token(
    auth_token: &str,
    scope: &BrowserStreamAuthScope,
    expires_at: i64,
) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"ctx-browser-stream|");
    hasher.update(scope.serialize().as_bytes());
    hasher.update(b"|");
    hasher.update(expires_at.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(auth_token.as_bytes());
    hex::encode(hasher.finalize())
}

pub(crate) fn derive_browser_query_secret(auth_token: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"ctx-desktop-browser-query-secret|");
    hasher.update(auth_token.as_bytes());
    hex::encode(hasher.finalize())
}

fn browser_query_secret_bearer_route_allowed(req: &Request<Body>) -> bool {
    req.uri().path().starts_with("/api/")
}

pub(super) fn browser_query_secret_bearer_is_valid(req: &Request<Body>, auth_token: &str) -> bool {
    browser_query_secret_bearer_route_allowed(req)
        && req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|value| value == derive_browser_query_secret(auth_token))
}

pub(super) fn browser_stream_query_token_is_valid(req: &Request<Body>, auth_token: &str) -> bool {
    if req.method() != Method::GET {
        return false;
    }
    let Some(scope) = browser_stream_scope(req) else {
        return false;
    };
    let Some(query_token) = query_param(req, "token") else {
        return false;
    };
    let Some(expires_at) = query_expires_at_within_window(
        req,
        BROWSER_STREAM_TOKEN_TTL_SECS,
        BROWSER_STREAM_TOKEN_MAX_PAST_SKEW_SECS,
        BROWSER_STREAM_TOKEN_MAX_FUTURE_SKEW_SECS,
    ) else {
        return false;
    };
    query_token == derive_browser_stream_token(auth_token, &scope, expires_at)
        || query_token
            == derive_browser_stream_token(
                &derive_browser_query_secret(auth_token),
                &scope,
                expires_at,
            )
}

const BROWSER_STREAM_TOKEN_TTL_SECS: i64 = 5 * 60;
const BROWSER_STREAM_TOKEN_MAX_PAST_SKEW_SECS: i64 = 10 * 60;
const BROWSER_STREAM_TOKEN_MAX_FUTURE_SKEW_SECS: i64 = 10 * 60;
const BROWSER_CAPABILITY_TOKEN_TTL_SECS: i64 = 60 * 60;
const BROWSER_CAPABILITY_TOKEN_MAX_FUTURE_SKEW_SECS: i64 = 60;

fn query_expires_at_within_window(
    req: &Request<Body>,
    ttl_secs: i64,
    max_past_skew_secs: i64,
    max_future_skew_secs: i64,
) -> Option<i64> {
    let expires_at = query_param(req, "expires_at").and_then(|value| value.parse().ok())?;
    let now = chrono::Utc::now().timestamp();
    if expires_at < now - max_past_skew_secs {
        return None;
    }
    if expires_at > now + ttl_secs + max_future_skew_secs {
        return None;
    }
    Some(expires_at)
}

pub(crate) fn derive_browser_capability_token(
    auth_token: &str,
    scope: &BrowserCapabilityAuthScope,
    expires_at: i64,
) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"ctx-browser-capability|");
    hasher.update(scope.serialize().as_bytes());
    hasher.update(b"|");
    hasher.update(expires_at.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(auth_token.as_bytes());
    hex::encode(hasher.finalize())
}

pub(super) fn browser_capability_query_token_is_valid(
    req: &Request<Body>,
    auth_token: &str,
) -> bool {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return false;
    }
    let Some(scope) = browser_capability_scope(req) else {
        return false;
    };
    let Some(query_token) = query_param(req, "token") else {
        return false;
    };
    let Some(expires_at) = query_expires_at_within_window(
        req,
        BROWSER_CAPABILITY_TOKEN_TTL_SECS,
        0,
        BROWSER_CAPABILITY_TOKEN_MAX_FUTURE_SKEW_SECS,
    ) else {
        return false;
    };
    query_token == derive_browser_capability_token(auth_token, &scope, expires_at)
        || query_token
            == derive_browser_capability_token(
                &derive_browser_query_secret(auth_token),
                &scope,
                expires_at,
            )
}
