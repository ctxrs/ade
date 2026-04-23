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

use crate::daemon::AppState;
use ctx_core::ids::ConnectionProfileId;

#[derive(Clone, Copy)]
pub(super) struct MobileAuthContext {
    pub(super) profile_id: ConnectionProfileId,
}

fn is_websocket_upgrade(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(axum::http::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
        || headers.contains_key("sec-websocket-key")
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
    if state.core.auth_token.is_none() {
        return Ok(next.run(req).await);
    }
    if req.extensions().get::<MobileAuthContext>().is_some() {
        return Ok(next.run(req).await);
    }

    let is_terminal_stream = path.starts_with("/api/terminals/") && path.ends_with("/stream");
    let is_ws = is_terminal_stream && is_websocket_upgrade(req.headers());
    let is_mobile_token_route = path == "/api/mobile/register";
    let header_token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.to_string());
    let query_token = req.uri().query().and_then(|q| {
        q.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            if k == "token" {
                Some(v.to_string())
            } else {
                None
            }
        })
    });
    let token = if is_ws {
        if header_token.is_some() {
            tracing::warn!(
                "Authorization header is deprecated for terminal websocket auth; use ?token="
            );
        }
        query_token.or(header_token)
    } else {
        header_token
    };

    if token.as_deref() == state.core.auth_token.as_deref() {
        return Ok(next.run(req).await);
    }
    if is_mobile_token_route {
        let Some(token_value) = token else {
            return Err(StatusCode::UNAUTHORIZED);
        };
        if let Some(profile_id) = verify_mobile_api_token(&state, &token_value).await? {
            req.extensions_mut()
                .insert(MobileAuthContext { profile_id });
            return Ok(next.run(req).await);
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}

pub(super) async fn verify_mobile_api_token(
    state: &Arc<AppState>,
    token: &str,
) -> Result<Option<ConnectionProfileId>, StatusCode> {
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
        Ok(Some(profile.id))
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
