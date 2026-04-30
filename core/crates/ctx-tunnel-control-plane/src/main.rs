use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use clap::Parser;
use ctx_tunnel_store::{CreateTunnelRequest, TunnelStore, TunnelStoreError};
use hmac::{Hmac, Mac};
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::net::SocketAddr;
use tracing::{info, warn};
use url::Url;
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
    client: reqwest::Client,
    config: ControlPlaneConfig,
}

#[derive(Clone)]
struct ControlPlaneConfig {
    supabase_url: String,
    supabase_anon_key: String,
    entitlements_url: String,
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

#[derive(Debug, Deserialize)]
struct EntitlementsSnapshot {
    #[allow(dead_code)]
    plan_type: String,
    features: serde_json::Value,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let database_url = std::env::var("MOBILE_TUNNEL_DATABASE_URL")
        .context("missing MOBILE_TUNNEL_DATABASE_URL")?;
    let supabase_url = std::env::var("SUPABASE_URL").context("missing SUPABASE_URL")?;
    let supabase_anon_key =
        std::env::var("SUPABASE_ANON_KEY").context("missing SUPABASE_ANON_KEY")?;
    let entitlements_url = std::env::var("CONTROL_PLANE_ENTITLEMENTS_URL").unwrap_or_else(|_| {
        format!(
            "{}/functions/v1/entitlements",
            supabase_url.trim_end_matches('/')
        )
    });
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
        client: reqwest::Client::new(),
        config: ControlPlaneConfig {
            supabase_url,
            supabase_anon_key,
            entitlements_url,
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
    tracing::info!("enable_mobile_access token_len={}", token.len());
    let user_id = fetch_user_id(&state, &token).await?;
    ensure_entitled(&state, &token).await?;

    if let Some(existing) = state
        .store
        .load_active_tunnel_for_user(&user_id)
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
            user_id,
            billing_subject_id: None,
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

    let user_id = match fetch_user_id(&state, &token).await {
        Ok(id) => id,
        Err((status, msg)) => return (status, Json(ApiErrorResp { error: msg })).into_response(),
    };

    let result = state.store.revoke_active_tunnels_for_user(&user_id).await;

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

async fn fetch_user_id(state: &AppState, token: &str) -> Result<String, (StatusCode, String)> {
    let url = format!(
        "{}/auth/v1/user",
        state.config.supabase_url.trim_end_matches('/')
    );
    let resp = state
        .client
        .get(url)
        .header("apikey", &state.config.supabase_anon_key)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_GATEWAY,
                "auth service unavailable".to_string(),
            )
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        warn!("auth user failed status={status} body={body}");
        return Err((StatusCode::UNAUTHORIZED, "invalid user session".to_string()));
    }

    let value = resp
        .json::<serde_json::Value>()
        .await
        .map_err(|_| (StatusCode::BAD_GATEWAY, "invalid auth response".to_string()))?;
    let user_id = value
        .get("user")
        .and_then(|user| user.get("id"))
        .or_else(|| value.get("id"))
        .and_then(|id| id.as_str());
    let Some(user_id) = user_id else {
        return Err((StatusCode::UNAUTHORIZED, "invalid user session".to_string()));
    };
    Ok(user_id.to_string())
}

async fn ensure_entitled(state: &AppState, token: &str) -> Result<(), (StatusCode, String)> {
    let url = Url::parse(&state.config.entitlements_url).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid entitlements url".to_string(),
        )
    })?;
    let resp = state
        .client
        .get(url)
        .header("apikey", &state.config.supabase_anon_key)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_GATEWAY,
                "entitlements unavailable".to_string(),
            )
        })?;

    if !resp.status().is_success() {
        return Err((StatusCode::FORBIDDEN, "entitlements required".to_string()));
    }

    let snapshot = resp.json::<EntitlementsSnapshot>().await.map_err(|_| {
        (
            StatusCode::BAD_GATEWAY,
            "invalid entitlements response".to_string(),
        )
    })?;

    let enabled = snapshot
        .features
        .get("remote_mobile_access")
        .and_then(|v| v.as_str())
        .map(|v| v == "enabled")
        .unwrap_or(false);

    if !enabled {
        return Err((
            StatusCode::FORBIDDEN,
            "mobile access not entitled".to_string(),
        ));
    }
    Ok(())
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
