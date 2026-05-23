use chrono::{DateTime, Utc};
use ctx_mobile_access_service::{
    persist_mobile_access_enable_bootstrap,
    route_contract::{
        DisableMobileAccessError, EnableMobileAccessRequest, EnableMobileAccessResult,
        MobileAccessRouteError, MobileAccessStatusSnapshot,
    },
    MobileAccessConfigSnapshot, PersistMobileAccessEnableBootstrapRequest,
};
use ctx_store::Store;
use ctx_transport_runtime::mobile_tunnel::MobileTunnelManager;
use serde_json::json;
use url::Url;

use super::control_plane::{
    request_control_plane_enable, revoke_control_plane_mobile_access_best_effort,
    ControlPlaneEnableResp, PAIRING_TOKEN_TTL_SECS,
};
use super::StartMobileTunnelRequest;

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
    store: &Store,
    mobile_tunnel: &MobileTunnelManager,
    daemon_url: &str,
    auth_token_configured: bool,
    request: EnableMobileAccessRequest,
) -> Result<EnableMobileAccessResult, MobileAccessRouteError> {
    if !auth_token_configured {
        return Err(MobileAccessRouteError::bad_request(
            "daemon auth token is not configured; refusing to expose daemon publicly",
        ));
    }

    let payload = request_control_plane_enable(&request.supabase_token).await?;
    let public_url = parse_allowed_public_url(&payload.public_base_url)?;
    let now = Utc::now();
    let (daemon_public_key, daemon_private_key) =
        ctx_transport_runtime::mobile_e2ee::generate_keypair();
    let bootstrap = persist_mobile_access_enable_bootstrap(
        store,
        PersistMobileAccessEnableBootstrapRequest {
            public_base_url: public_url.as_str().trim_end_matches('/').to_string(),
            relay_base_url: payload.relay_base_url.clone(),
            tunnel_id: payload.tunnel_id.clone(),
            tunnel_secret: payload.tunnel_secret.clone(),
            daemon_public_key,
            daemon_private_key,
            now,
            pairing_token_ttl_seconds: PAIRING_TOKEN_TTL_SECS,
        },
    )
    .await?;
    super::start_mobile_tunnel_best_effort(
        mobile_tunnel,
        daemon_url,
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
        &bootstrap.config,
        bootstrap.pairing_token,
        bootstrap.pairing_expires_at,
    ))
}

pub(super) async fn disable_mobile_access_for_route(
    store: &Store,
    mobile_tunnel: &MobileTunnelManager,
    supabase_token: String,
) -> Result<(), DisableMobileAccessError> {
    revoke_control_plane_mobile_access_best_effort(&supabase_token).await;
    super::disable_mobile_access_runtime(store, mobile_tunnel).await
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

fn build_enable_mobile_access_result(
    payload: ControlPlaneEnableResp,
    config: &MobileAccessConfigSnapshot,
    pairing_token: String,
    pairing_expires_at: DateTime<Utc>,
) -> EnableMobileAccessResult {
    let status = MobileAccessStatusSnapshot {
        enabled: true,
        tunnel_id: Some(payload.tunnel_id.clone()),
        public_base_url: Some(config.public_base_url.clone()),
        relay_base_url: Some(payload.relay_base_url),
        daemon_public_key: Some(config.daemon_public_key.clone()),
        tunnel_state: ctx_transport_runtime::mobile_tunnel::MobileTunnelState::Running,
        last_error: None,
    };

    let qr_payload = json!({
        "type": "context_mobile_e2ee",
        "version": 1,
        "tunnel_id": payload.tunnel_id,
        "base_url": config.public_base_url,
        "pairing_token": pairing_token,
        "daemon_public_key": config.daemon_public_key,
        "pairing_request_encryption": ctx_transport_runtime::mobile_e2ee::PAIRING_REQUEST_ENCRYPTION,
    });

    EnableMobileAccessResult {
        status,
        qr_payload,
        pairing_expires_at,
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
