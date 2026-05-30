use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use clap::Parser;
use ctx_tunnel_store::{CreateTunnelRequest, TunnelStore, TunnelStoreError};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "ctx-tunnel-control-plane", version)]
struct Args {
    #[arg(long, default_value = "0.0.0.0:8791")]
    listen: String,
}

#[derive(Clone)]
struct AppState {
    store: TunnelStore,
    config: ControlPlaneConfig,
}

#[derive(Clone)]
struct ControlPlaneConfig {
    master_secret: Vec<u8>,
    public_base_url: String,
    relay_region: String,
}

#[derive(Debug, Serialize)]
struct ApiErrorResp {
    error: String,
}

#[derive(Debug, Serialize)]
struct EnableMobileAccessResp {
    tunnel_id: String,
    public_base_url: String,
    relay_base_url: String,
    tunnel_secret: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let database_url = std::env::var("MOBILE_TUNNEL_DATABASE_URL")
        .context("missing MOBILE_TUNNEL_DATABASE_URL")?;
    let public_base_url = std::env::var("MOBILE_TUNNEL_PUBLIC_BASE_URL")
        .context("missing MOBILE_TUNNEL_PUBLIC_BASE_URL")?;
    let relay_region = std::env::var("MOBILE_TUNNEL_RELAY_REGION")
        .context("missing MOBILE_TUNNEL_RELAY_REGION")?;
    let master_secret = load_master_secret()?;

    let store = TunnelStore::connect(&database_url)
        .await
        .context("connecting to mobile tunnel database")?;

    let state = AppState {
        store,
        config: ControlPlaneConfig {
            master_secret,
            public_base_url,
            relay_region,
        },
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/mobile/enable", post(enable_mobile_access))
        .route("/v1/mobile/revoke", post(revoke_mobile_access))
        .with_state(state);

    let addr: SocketAddr = args.listen.parse().context("parsing --listen")?;
    info!("control plane listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("binding listener")?;
    axum::serve(listener, app)
        .await
        .context("serving control plane")?;
    Ok(())
}

async fn health(State(state): State<AppState>) -> Response {
    match state.store.health_snapshot().await {
        Ok(snapshot) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "relay_count": snapshot.relay_count,
                "healthy_relay_count": snapshot.healthy_relay_count,
            })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "ok": false,
                "error": err.to_string(),
            })),
        )
            .into_response(),
    }
}

async fn enable_mobile_access(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Response {
    match enable_mobile_access_inner(state, headers).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err((status, msg)) => (status, Json(ApiErrorResp { error: msg })).into_response(),
    }
}

async fn enable_mobile_access_inner(
    state: AppState,
    headers: axum::http::HeaderMap,
) -> Result<EnableMobileAccessResp, (StatusCode, String)> {
    let token = extract_bearer(&headers).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            "authorization required".to_string(),
        )
    })?;
    let binding = extract_tunnel_grant_binding(&headers)?;
    let grant = verify_managed_tunnel_grant(&state, &token, &binding).await?;

    if let Some(existing) = state
        .store
        .load_active_tunnel_for_binding(&grant.user_id, &grant.daemon_id, &grant.device_id)
        .await
        .map_err(store_api_error)?
    {
        return tunnel_assignment_to_response(&state, existing);
    }

    let tunnel_id = Uuid::new_v4().to_string();
    let relay = state
        .store
        .assign_relay(&state.config.relay_region)
        .await
        .map_err(store_api_error)?;
    let public_base_url = normalize_public_base_url(&state.config.public_base_url, &tunnel_id);

    let assignment = state
        .store
        .create_tunnel(CreateTunnelRequest {
            tunnel_id,
            user_id: grant.user_id,
            billing_subject_id: Some(grant.billing_subject_id),
            grant_id: grant.grant_id,
            daemon_id: grant.daemon_id,
            device_id: grant.device_id,
            relay_id: relay.relay_id,
            public_base_url,
        })
        .await
        .map_err(store_api_error)?;
    tunnel_assignment_to_response(&state, assignment)
}

async fn revoke_mobile_access(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Response {
    let token = match extract_bearer(&headers) {
        Some(token) => token,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResp {
                    error: "authorization required".into(),
                }),
            )
                .into_response()
        }
    };

    let binding = match extract_tunnel_grant_binding(&headers) {
        Ok(binding) => binding,
        Err((status, msg)) => return (status, Json(ApiErrorResp { error: msg })).into_response(),
    };
    let grant = match verify_managed_tunnel_grant(&state, &token, &binding).await {
        Ok(grant) => grant,
        Err((status, msg)) => return (status, Json(ApiErrorResp { error: msg })).into_response(),
    };

    let result = state
        .store
        .revoke_active_tunnels_for_binding(&grant.user_id, &grant.daemon_id, &grant.device_id)
        .await;

    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => {
            warn!("failed to revoke tunnels: {err}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to revoke tunnel".into(),
                }),
            )
                .into_response()
        }
    }
}

fn tunnel_assignment_to_response(
    state: &AppState,
    assignment: ctx_tunnel_store::TunnelAssignment,
) -> Result<EnableMobileAccessResp, (StatusCode, String)> {
    let tunnel_id = assignment.tunnel_id;
    let public_base_url = normalize_public_base_url(&assignment.public_base_url, &tunnel_id);
    let tunnel_secret = derive_tunnel_secret(&state.config.master_secret, &tunnel_id)?;
    Ok(EnableMobileAccessResp {
        tunnel_id,
        relay_base_url: assignment.relay_public_base_url,
        public_base_url,
        tunnel_secret,
    })
}

async fn verify_managed_tunnel_grant(
    state: &AppState,
    token: &str,
    binding: &TunnelGrantBinding,
) -> Result<ctx_tunnel_store::VerifiedMobileTunnelGrant, (StatusCode, String)> {
    let trimmed = token.trim();
    if !trimmed.starts_with("ctmt_") {
        return Err((
            StatusCode::UNAUTHORIZED,
            "invalid managed tunnel grant".to_string(),
        ));
    }
    let digest = managed_tunnel_grant_digest(trimmed);
    state
        .store
        .verify_mobile_tunnel_grant(&digest, &binding.daemon_id, &binding.device_id)
        .await
        .map_err(store_api_error)?
        .ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                "mobile access not entitled".to_string(),
            )
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TunnelGrantBinding {
    daemon_id: String,
    device_id: String,
}

fn extract_tunnel_grant_binding(
    headers: &axum::http::HeaderMap,
) -> Result<TunnelGrantBinding, (StatusCode, String)> {
    Ok(TunnelGrantBinding {
        daemon_id: required_header(headers, "x-ctx-daemon-id")?,
        device_id: required_header(headers, "x-ctx-device-id")?,
    })
}

fn required_header(
    headers: &axum::http::HeaderMap,
    name: &'static str,
) -> Result<String, (StatusCode, String)> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, format!("{name} required")))
}

fn managed_tunnel_grant_digest(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(|v| v.trim().to_string())
}

fn normalize_public_base_url(base: &str, tunnel_id: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.contains("/t/") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/t/{tunnel_id}")
    }
}

fn derive_tunnel_secret(master: &[u8], tunnel_id: &str) -> Result<String, (StatusCode, String)> {
    let mut mac = Hmac::<Sha256>::new_from_slice(master).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid master secret".to_string(),
        )
    })?;
    mac.update(tunnel_id.as_bytes());
    let digest = mac.finalize().into_bytes();
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    Ok(encoded)
}

fn load_master_secret() -> anyhow::Result<Vec<u8>> {
    let raw =
        std::env::var("CTX_TUNNEL_MASTER_SECRET").context("missing CTX_TUNNEL_MASTER_SECRET")?;
    parse_master_secret(&raw)
}

fn parse_master_secret(raw: &str) -> anyhow::Result<Vec<u8>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("CTX_TUNNEL_MASTER_SECRET must not be empty");
    }
    Ok(trimmed.as_bytes().to_vec())
}

fn store_api_error(err: TunnelStoreError) -> (StatusCode, String) {
    match err {
        TunnelStoreError::NoHealthyRelay { .. } => {
            warn!("no healthy relay available: {err}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "no healthy relay available".to_string(),
            )
        }
        _ => {
            warn!("mobile tunnel store error: {err}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "mobile tunnel store unavailable".to_string(),
            )
        }
    }
}

#[cfg(test)]
mod tests;
