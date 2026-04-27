use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::{Method, Request};
use axum::middleware::Next;
use axum::response::IntoResponse;
use base64::Engine;
use rand_core::RngCore;
use sha2::Digest;
use url::form_urlencoded;

use crate::daemon::AppState;
use ctx_core::ids::{ConnectionProfileId, SessionId};
use ctx_core::models::MobileConnectionProfile;

use super::{
    default_mobile_profile_scopes, mobile_scope_set_from_strings, MobileScope, MobileScopeSet,
};

#[derive(Clone, Copy)]
pub(super) struct MobileAuthContext {
    pub(super) profile_id: ConnectionProfileId,
    scopes: MobileScopeSet,
}

impl MobileAuthContext {
    pub(super) fn allows(self, scope: MobileScope) -> bool {
        self.scopes.allows(scope)
    }
}

fn mobile_auth_context_from_profile(
    profile: &MobileConnectionProfile,
) -> Result<MobileAuthContext, String> {
    let scopes = mobile_scope_set_from_strings(&profile.scopes)?;
    Ok(MobileAuthContext {
        profile_id: profile.id,
        scopes,
    })
}

fn log_invalid_mobile_scope_set(profile_id: ConnectionProfileId, error: &str) {
    tracing::warn!(
        profile_id = %profile_id.0,
        error,
        "rejecting mobile profile with invalid scope configuration"
    );
}

fn mobile_profile_uses_legacy_empty_scope_shape(profile: &MobileConnectionProfile) -> bool {
    profile.scopes.iter().all(|scope| scope.trim().is_empty())
}

async fn migrate_legacy_mobile_profile_scopes(
    state: &Arc<AppState>,
    profile: &MobileConnectionProfile,
) -> Result<MobileAuthContext, StatusCode> {
    let scopes = default_mobile_profile_scopes();
    state
        .global_store()
        .update_mobile_connection_profile_scopes(profile.id, scopes.clone())
        .await
        .map_err(|e| {
            tracing::error!(
                profile_id = %profile.id.0,
                "failed to migrate legacy mobile profile scopes: {e:?}"
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    tracing::info!(
        profile_id = %profile.id.0,
        "migrated legacy mobile profile to explicit default scopes"
    );
    let scopes = mobile_scope_set_from_strings(&scopes).map_err(|error| {
        tracing::error!(
            profile_id = %profile.id.0,
            error,
            "default mobile scope bundle became invalid"
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(MobileAuthContext {
        profile_id: profile.id,
        scopes,
    })
}

async fn resolve_mobile_auth_context(
    state: &Arc<AppState>,
    profile: MobileConnectionProfile,
) -> Result<Option<MobileAuthContext>, StatusCode> {
    match mobile_auth_context_from_profile(&profile) {
        Ok(auth) => Ok(Some(auth)),
        Err(error) => {
            if mobile_profile_uses_legacy_empty_scope_shape(&profile) {
                let auth = migrate_legacy_mobile_profile_scopes(state, &profile).await?;
                Ok(Some(auth))
            } else {
                log_invalid_mobile_scope_set(profile.id, &error);
                Ok(None)
            }
        }
    }
}

pub(super) async fn load_mobile_auth_context_for_profile(
    state: &Arc<AppState>,
    profile_id: ConnectionProfileId,
) -> Result<Option<MobileAuthContext>, StatusCode> {
    let profile = state
        .global_store()
        .get_mobile_connection_profile(profile_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to load mobile connection profile: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    match profile {
        Some(profile) => resolve_mobile_auth_context(state, profile).await,
        None => Ok(None),
    }
}

#[derive(Clone, Copy)]
enum ScopedMcpRoute {
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

fn is_websocket_upgrade(headers: &axum::http::HeaderMap) -> bool {
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

fn scoped_mcp_route(req: &Request<Body>) -> Option<ScopedMcpRoute> {
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
) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"ctx-browser-stream|");
    hasher.update(scope.serialize().as_bytes());
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

fn browser_stream_query_token_is_valid(req: &Request<Body>, auth_token: &str) -> bool {
    let Some(scope) = browser_stream_scope(req) else {
        return false;
    };
    let Some(query_token) = query_param(req, "token") else {
        return false;
    };
    query_token == derive_browser_stream_token(auth_token, &scope)
        || query_token
            == derive_browser_stream_token(&derive_browser_query_secret(auth_token), &scope)
}

const BROWSER_CAPABILITY_TOKEN_TTL_SECS: i64 = 60 * 60;
const BROWSER_CAPABILITY_TOKEN_MAX_FUTURE_SKEW_SECS: i64 = 60;

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

fn browser_capability_query_token_is_valid(req: &Request<Body>, auth_token: &str) -> bool {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return false;
    }
    let Some(scope) = browser_capability_scope(req) else {
        return false;
    };
    let Some(query_token) = query_param(req, "token") else {
        return false;
    };
    let Some(expires_at) = query_param(req, "expires_at").and_then(|value| value.parse().ok())
    else {
        return false;
    };
    let now = chrono::Utc::now().timestamp();
    if expires_at < now {
        return false;
    }
    if expires_at
        > now + BROWSER_CAPABILITY_TOKEN_TTL_SECS + BROWSER_CAPABILITY_TOKEN_MAX_FUTURE_SKEW_SECS
    {
        return false;
    }
    query_token == derive_browser_capability_token(auth_token, &scope, expires_at)
        || query_token
            == derive_browser_capability_token(
                &derive_browser_query_secret(auth_token),
                &scope,
                expires_at,
            )
}

pub(super) async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request<Body>,
    next: Next,
) -> Result<impl IntoResponse, StatusCode> {
    if req.method() == Method::OPTIONS {
        return Ok(next.run(req).await);
    }
    let path = req.uri().path();
    if !path.starts_with("/api/") || path == "/api/health" {
        return Ok(next.run(req).await);
    }
    if path.starts_with("/api/mobile/secure") || path == "/api/mobile/pair" {
        return Ok(next.run(req).await);
    }
    if req.extensions().get::<MobileAuthContext>().is_some() {
        return Ok(next.run(req).await);
    }
    if state.core.auth_token.is_none() {
        return Ok(next.run(req).await);
    }
    let is_terminal_stream = path.starts_with("/api/terminals/") && path.ends_with("/stream");
    let is_ws = is_terminal_stream && is_websocket_upgrade(req.headers());
    if is_ws {
        return Ok(next.run(req).await);
    }
    let is_mobile_token_route = path == "/api/mobile/register";
    let header_token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.to_string());
    let token = header_token;

    if token.as_deref() == state.core.auth_token.as_deref() {
        return Ok(next.run(req).await);
    }
    if let Some(auth_token) = state.core.auth_token.as_deref() {
        if browser_stream_query_token_is_valid(&req, auth_token) {
            return Ok(next.run(req).await);
        }
        if browser_capability_query_token_is_valid(&req, auth_token) {
            return Ok(next.run(req).await);
        }
    }
    if let Some(token_value) = token.as_deref() {
        if let Some(route) = scoped_mcp_route(&req) {
            if let Some(mcp_auth) = crate::daemon::verify_mcp_auth_token(&state, token_value).await
            {
                let allowed = match route {
                    ScopedMcpRoute::SessionSubagents { session_id } => {
                        mcp_auth.allows_subagents(session_id)
                    }
                    ScopedMcpRoute::SessionArtifacts { session_id } => {
                        mcp_auth.allows_artifacts(session_id)
                    }
                    ScopedMcpRoute::MergeQueueSubmit => mcp_auth.capabilities.merge_queue_submit,
                };
                if allowed {
                    req.extensions_mut().insert(mcp_auth);
                    return Ok(next.run(req).await);
                }
            }
        }
    }
    if is_mobile_token_route {
        let Some(token_value) = token else {
            return Err(StatusCode::UNAUTHORIZED);
        };
        if let Some(mobile_auth) = verify_mobile_api_token(&state, &token_value).await? {
            req.extensions_mut().insert(mobile_auth);
            return Ok(next.run(req).await);
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}

pub(super) async fn verify_mobile_api_token(
    state: &Arc<AppState>,
    token: &str,
) -> Result<Option<MobileAuthContext>, StatusCode> {
    let hash = hash_api_token(token);
    let profile = state
        .global_store()
        .get_mobile_connection_profile_by_token_hash(&hash)
        .await
        .map_err(|e| {
            tracing::error!("failed to query mobile connection profile: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    if let Some(profile) = profile {
        if let Err(err) = state
            .global_store()
            .mark_mobile_connection_profile_used(profile.id)
            .await
        {
            tracing::warn!("failed to update profile usage: {err:?}");
        }
        resolve_mobile_auth_context(state, profile).await
    } else {
        Ok(None)
    }
}

pub(super) fn hash_api_token(token: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub(super) fn hash_pairing_token(token: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub(super) fn generate_mobile_api_token() -> String {
    format!("ctxm_{}", uuid::Uuid::new_v4().to_string().replace('-', ""))
}

pub(super) fn generate_pairing_token() -> String {
    let mut bytes = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
