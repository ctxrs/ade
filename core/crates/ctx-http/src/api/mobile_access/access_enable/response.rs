use super::pairing::MobilePairingBootstrap;
use super::profile_config::ManagedMobileAccessKeys;
use super::*;

pub(super) async fn start_mobile_tunnel_best_effort(
    state: &Arc<AppState>,
    payload: &ControlPlaneEnableResp,
    public_url: &Url,
) {
    let tunnel_cfg = mobile_tunnel::StartMobileTunnelConfig {
        relay_base_url: payload.relay_base_url.clone(),
        tunnel_id: payload.tunnel_id.clone(),
        tunnel_secret: payload.tunnel_secret.clone(),
        public_base_url: public_url.as_str().trim_end_matches('/').to_string(),
        local_daemon_url: state.core.daemon_url.trim_end_matches('/').to_string(),
    };
    if let Err(e) = state.transport.mobile_tunnel.start(tunnel_cfg).await {
        tracing::warn!("failed to start mobile tunnel: {e:#}");
    }
}

pub(super) fn build_enable_mobile_access_response(
    payload: ControlPlaneEnableResp,
    public_url: &Url,
    keys: &ManagedMobileAccessKeys,
    pairing: MobilePairingBootstrap,
) -> EnableMobileAccessResp {
    let public_base_url = public_url.as_str().trim_end_matches('/').to_string();
    let status = MobileAccessStatus {
        enabled: true,
        tunnel_id: Some(payload.tunnel_id.clone()),
        public_base_url: Some(public_base_url.clone()),
        relay_base_url: Some(payload.relay_base_url),
        daemon_public_key: Some(keys.daemon_public_key.clone()),
        tunnel_state: mobile_tunnel::MobileTunnelState::Running,
        last_error: None,
    };

    let qr_payload = serde_json::json!({
        "type": "context_mobile_e2ee",
        "version": 1,
        "tunnel_id": payload.tunnel_id,
        "base_url": public_base_url,
        "pairing_token": pairing.pairing_token,
        "daemon_public_key": keys.daemon_public_key,
        "pairing_request_encryption": mobile_e2ee::PAIRING_REQUEST_ENCRYPTION,
    });

    EnableMobileAccessResp {
        status,
        qr_payload,
        pairing_expires_at: pairing.expires_at.to_rfc3339(),
    }
}
