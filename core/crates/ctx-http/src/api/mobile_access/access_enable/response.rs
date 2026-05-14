use super::pairing::MobilePairingBootstrap;
use super::profile_config::ManagedMobileAccessKeys;
use super::*;
use crate::daemon::mobile_access as daemon_mobile_access;

pub(super) async fn start_mobile_tunnel_best_effort(
    state: &CoreHandle,
    payload: &ControlPlaneEnableResp,
    public_url: &Url,
) {
    let request = daemon_mobile_access::StartMobileTunnelRequest {
        relay_base_url: payload.relay_base_url.clone(),
        tunnel_id: payload.tunnel_id.clone(),
        tunnel_secret: payload.tunnel_secret.clone(),
        public_base_url: public_url.as_str().trim_end_matches('/').to_string(),
    };
    state.start_mobile_tunnel_best_effort(request).await;
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
        tunnel_state: ctx_transport_runtime::mobile_tunnel::MobileTunnelState::Running,
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
