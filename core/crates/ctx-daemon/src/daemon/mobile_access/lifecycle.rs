use std::sync::Arc;

use chrono::{DateTime, Utc};
use ctx_mobile_access_service::{MobileAccessServiceError, MobileAccessServiceErrorKind};
use serde_json::json;
use url::Url;

use super::control_plane::{
    request_control_plane_enable, revoke_control_plane_mobile_access_best_effort,
    ControlPlaneEnableResp, PAIRING_TOKEN_TTL_SECS,
};
use super::tokens::{
    generate_mobile_api_token, generate_pairing_token, hash_api_token, hash_pairing_token,
};
use super::{
    default_mobile_profile_scopes, DisableMobileAccessError, EnableMobileAccessRequest,
    MobileAccessConfigUpsert, MobileAccessStatusSnapshot, StartMobileTunnelRequest,
};
use crate::daemon::DaemonState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileAccessRouteErrorKind {
    BadRequest,
    Unauthorized,
    Forbidden,
    Conflict,
    NotFound,
    BadGateway,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileAccessRouteError {
    kind: MobileAccessRouteErrorKind,
    message: String,
}

impl MobileAccessRouteError {
    pub fn new(kind: MobileAccessRouteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(MobileAccessRouteErrorKind::BadRequest, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(MobileAccessRouteErrorKind::Unauthorized, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(MobileAccessRouteErrorKind::Internal, message)
    }

    pub fn kind(&self) -> MobileAccessRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<MobileAccessServiceError> for MobileAccessRouteError {
    fn from(error: MobileAccessServiceError) -> Self {
        let kind = match error.kind() {
            MobileAccessServiceErrorKind::BadRequest => MobileAccessRouteErrorKind::BadRequest,
            MobileAccessServiceErrorKind::Unauthorized => MobileAccessRouteErrorKind::Unauthorized,
            MobileAccessServiceErrorKind::NotFound => MobileAccessRouteErrorKind::NotFound,
            MobileAccessServiceErrorKind::Internal => MobileAccessRouteErrorKind::Internal,
        };
        Self::new(kind, error.message())
    }
}

#[derive(Debug, Clone)]
pub struct EnableMobileAccessResult {
    pub status: MobileAccessStatusSnapshot,
    pub qr_payload: serde_json::Value,
    pub pairing_expires_at: DateTime<Utc>,
}

struct ManagedMobileAccessKeys {
    daemon_public_key: String,
    daemon_private_key: String,
    profile_id: ctx_core::ids::ConnectionProfileId,
    created_at: DateTime<Utc>,
}

struct MobilePairingBootstrap {
    pairing_token: String,
    expires_at: DateTime<Utc>,
}

pub fn mobile_public_url_is_allowed(url: &Url) -> bool {
    if url.scheme() == "https" {
        return true;
    }
    if url.scheme() != "http" {
        return false;
    }
    if std::env::var_os("CTX_MOBILE_TUNNEL_ALLOW_INSECURE_LOOPBACK").is_none() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .map(|addr| addr.is_loopback())
            .unwrap_or(false)
}

pub(super) async fn enable_mobile_access_for_route(
    state: &Arc<DaemonState>,
    request: EnableMobileAccessRequest,
) -> Result<EnableMobileAccessResult, MobileAccessRouteError> {
    if state.core.auth_token.is_none() {
        return Err(MobileAccessRouteError::bad_request(
            "daemon auth token is not configured; refusing to expose daemon publicly",
        ));
    }

    let payload = request_control_plane_enable(&request.supabase_token).await?;
    let public_url = parse_allowed_public_url(&payload.public_base_url)?;
    let now = Utc::now();
    let keys = load_or_create_managed_mobile_access_keys(state, &public_url, now).await?;
    persist_mobile_access_config(state, &payload, &public_url, &keys, now).await?;
    let pairing = create_mobile_pairing_bootstrap(state, now).await?;
    super::start_mobile_tunnel_best_effort(
        state,
        StartMobileTunnelRequest {
            relay_base_url: payload.relay_base_url.clone(),
            tunnel_id: payload.tunnel_id.clone(),
            tunnel_secret: payload.tunnel_secret.clone(),
            public_base_url: public_url.as_str().trim_end_matches('/').to_string(),
        },
    )
    .await;

    Ok(build_enable_mobile_access_result(
        payload,
        &public_url,
        &keys,
        pairing,
    ))
}

pub(super) async fn disable_mobile_access_for_route(
    state: &Arc<DaemonState>,
    supabase_token: String,
) -> Result<(), DisableMobileAccessError> {
    revoke_control_plane_mobile_access_best_effort(&supabase_token).await;
    super::disable_mobile_access_runtime(state).await
}

fn parse_allowed_public_url(raw_public_base_url: &str) -> Result<Url, MobileAccessRouteError> {
    let public_url = Url::parse(raw_public_base_url)
        .map_err(|_| MobileAccessRouteError::bad_request("public_base_url must be a valid URL"))?;
    if !mobile_public_url_is_allowed(&public_url) {
        return Err(MobileAccessRouteError::bad_request(
            "public_base_url must use https://",
        ));
    }
    Ok(public_url)
}

async fn load_or_create_managed_mobile_access_keys(
    state: &Arc<DaemonState>,
    public_url: &Url,
    now: DateTime<Utc>,
) -> Result<ManagedMobileAccessKeys, MobileAccessRouteError> {
    match state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            MobileAccessRouteError::internal("failed to read mobile access config")
        })? {
        Some(cfg) => {
            ensure_managed_profile_scopes(state, cfg.profile_id).await?;
            Ok(ManagedMobileAccessKeys {
                daemon_public_key: cfg.daemon_public_key,
                daemon_private_key: cfg.daemon_private_key,
                profile_id: cfg.profile_id,
                created_at: cfg.created_at,
            })
        }
        None => create_managed_mobile_access_keys(state, public_url, now).await,
    }
}

async fn ensure_managed_profile_scopes(
    state: &Arc<DaemonState>,
    profile_id: ctx_core::ids::ConnectionProfileId,
) -> Result<(), MobileAccessRouteError> {
    let profile = state
        .global_store()
        .get_mobile_connection_profile(profile_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to read managed mobile profile: {e:?}");
            MobileAccessRouteError::internal("failed to read managed profile")
        })?
        .ok_or_else(|| MobileAccessRouteError::internal("managed mobile profile is missing"))?;
    if profile.scopes.is_empty() {
        state
            .global_store()
            .update_mobile_connection_profile_scopes(profile.id, default_mobile_profile_scopes())
            .await
            .map_err(|e| {
                tracing::error!("failed to backfill managed mobile profile scopes: {e:?}");
                MobileAccessRouteError::internal("failed to update managed profile")
            })?;
    }
    Ok(())
}

async fn create_managed_mobile_access_keys(
    state: &Arc<DaemonState>,
    public_url: &Url,
    now: DateTime<Utc>,
) -> Result<ManagedMobileAccessKeys, MobileAccessRouteError> {
    let (public_key, private_key) = ctx_transport_runtime::mobile_e2ee::generate_keypair();
    let token = generate_mobile_api_token();
    let token_hash = hash_api_token(&token);
    let token_prefix: String = token.chars().take(8).collect();
    let profile = state
        .global_store()
        .create_mobile_connection_profile(
            "Managed Mobile Access".to_string(),
            public_url.as_str().trim_end_matches('/').to_string(),
            token_hash,
            token_prefix,
            default_mobile_profile_scopes(),
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to create managed mobile profile: {e:?}");
            MobileAccessRouteError::internal("failed to create managed profile")
        })?;
    Ok(ManagedMobileAccessKeys {
        daemon_public_key: public_key,
        daemon_private_key: private_key,
        profile_id: profile.id,
        created_at: now,
    })
}

async fn persist_mobile_access_config(
    state: &Arc<DaemonState>,
    payload: &ControlPlaneEnableResp,
    public_url: &Url,
    keys: &ManagedMobileAccessKeys,
    now: DateTime<Utc>,
) -> Result<(), MobileAccessRouteError> {
    let config = MobileAccessConfigUpsert {
        profile_id: keys.profile_id,
        tunnel_id: payload.tunnel_id.clone(),
        public_base_url: public_url.as_str().trim_end_matches('/').to_string(),
        relay_base_url: payload.relay_base_url.clone(),
        tunnel_secret: payload.tunnel_secret.clone(),
        daemon_public_key: keys.daemon_public_key.clone(),
        daemon_private_key: keys.daemon_private_key.clone(),
        enabled: true,
        created_at: keys.created_at,
        updated_at: now,
    };

    state
        .global_store()
        .upsert_mobile_access_config(config.into_store_config())
        .await
        .map_err(|e| {
            tracing::error!("failed to persist mobile access config: {e:?}");
            MobileAccessRouteError::internal("failed to persist mobile access config")
        })?;

    Ok(())
}

async fn create_mobile_pairing_bootstrap(
    state: &Arc<DaemonState>,
    now: DateTime<Utc>,
) -> Result<MobilePairingBootstrap, MobileAccessRouteError> {
    let pairing_token = generate_pairing_token();
    let pairing_hash = hash_pairing_token(&pairing_token);
    let expires_at = now + chrono::Duration::seconds(PAIRING_TOKEN_TTL_SECS);
    state
        .global_store()
        .insert_mobile_pairing_token(&uuid::Uuid::new_v4().to_string(), &pairing_hash, expires_at)
        .await
        .map_err(|e| {
            tracing::error!("failed to persist pairing token: {e:?}");
            MobileAccessRouteError::internal("failed to persist pairing token")
        })?;
    Ok(MobilePairingBootstrap {
        pairing_token,
        expires_at,
    })
}

fn build_enable_mobile_access_result(
    payload: ControlPlaneEnableResp,
    public_url: &Url,
    keys: &ManagedMobileAccessKeys,
    pairing: MobilePairingBootstrap,
) -> EnableMobileAccessResult {
    let public_base_url = public_url.as_str().trim_end_matches('/').to_string();
    let status = MobileAccessStatusSnapshot {
        enabled: true,
        tunnel_id: Some(payload.tunnel_id.clone()),
        public_base_url: Some(public_base_url.clone()),
        relay_base_url: Some(payload.relay_base_url),
        daemon_public_key: Some(keys.daemon_public_key.clone()),
        tunnel_state: ctx_transport_runtime::mobile_tunnel::MobileTunnelState::Running,
        last_error: None,
    };

    let qr_payload = json!({
        "type": "context_mobile_e2ee",
        "version": 1,
        "tunnel_id": payload.tunnel_id,
        "base_url": public_base_url,
        "pairing_token": pairing.pairing_token,
        "daemon_public_key": keys.daemon_public_key,
        "pairing_request_encryption": ctx_transport_runtime::mobile_e2ee::PAIRING_REQUEST_ENCRYPTION,
    });

    EnableMobileAccessResult {
        status,
        qr_payload,
        pairing_expires_at: pairing.expires_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    #[test]
    fn mobile_public_url_requires_https_unless_explicit_loopback_test_flag_is_set() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let previous = std::env::var("CTX_MOBILE_TUNNEL_ALLOW_INSECURE_LOOPBACK").ok();
        std::env::remove_var("CTX_MOBILE_TUNNEL_ALLOW_INSECURE_LOOPBACK");

        assert!(mobile_public_url_is_allowed(
            &Url::parse("https://tunnel.ctx.rs/t/id").unwrap()
        ));
        assert!(!mobile_public_url_is_allowed(
            &Url::parse("http://127.0.0.1:8790/t/id").unwrap()
        ));

        std::env::set_var("CTX_MOBILE_TUNNEL_ALLOW_INSECURE_LOOPBACK", "1");
        assert!(mobile_public_url_is_allowed(
            &Url::parse("http://127.0.0.1:8790/t/id").unwrap()
        ));
        assert!(mobile_public_url_is_allowed(
            &Url::parse("http://localhost:8790/t/id").unwrap()
        ));
        assert!(!mobile_public_url_is_allowed(
            &Url::parse("http://example.com/t/id").unwrap()
        ));

        match previous {
            Some(value) => std::env::set_var("CTX_MOBILE_TUNNEL_ALLOW_INSECURE_LOOPBACK", value),
            None => std::env::remove_var("CTX_MOBILE_TUNNEL_ALLOW_INSECURE_LOOPBACK"),
        }
    }
}
