use ctx_mobile_access_service::route_contract::{
    MobileAccessRouteError, MobileAccessRouteErrorKind,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const DEFAULT_TUNNEL_CONTROL_PLANE_URL: &str = "https://tunnel.ctx.rs";

pub(super) const PAIRING_TOKEN_TTL_SECS: i64 = 10 * 60;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ControlPlaneEnableResp {
    pub(super) tunnel_id: String,
    pub(super) public_base_url: String,
    pub(super) relay_base_url: String,
    pub(super) tunnel_secret: String,
}

pub(super) fn resolve_control_plane_url() -> String {
    std::env::var("CTX_TUNNEL_CONTROL_PLANE_URL")
        .unwrap_or_else(|_| DEFAULT_TUNNEL_CONTROL_PLANE_URL.to_string())
}

pub(super) async fn request_control_plane_enable(
    managed_tunnel_grant: &str,
) -> Result<ControlPlaneEnableResp, MobileAccessRouteError> {
    let control_plane_url = resolve_control_plane_url();
    if control_plane_url.trim().is_empty() {
        return Err(MobileAccessRouteError::new(
            MobileAccessRouteErrorKind::BadRequest,
            "CTX_TUNNEL_CONTROL_PLANE_URL is not set",
        ));
    }
    let binding = managed_tunnel_binding(managed_tunnel_grant)?;

    let enable_resp = reqwest::Client::new()
        .post(format!(
            "{}/v1/mobile/enable",
            control_plane_url.trim_end_matches('/')
        ))
        .bearer_auth(managed_tunnel_grant.trim())
        .header("x-ctx-daemon-id", binding.daemon_id)
        .header("x-ctx-device-id", binding.device_id)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("failed to call control plane: {e:?}");
            MobileAccessRouteError::new(
                MobileAccessRouteErrorKind::BadGateway,
                "failed to reach control plane",
            )
        })?;

    if !enable_resp.status().is_success() {
        let status = enable_resp.status();
        let body = enable_resp.text().await.unwrap_or_default();
        tracing::warn!("control plane denied enable: {status} {body}");
        return Err(MobileAccessRouteError::new(
            MobileAccessRouteErrorKind::Forbidden,
            "mobile access not entitled",
        ));
    }

    enable_resp
        .json::<ControlPlaneEnableResp>()
        .await
        .map_err(|e| {
            tracing::error!("invalid control plane response: {e:?}");
            MobileAccessRouteError::new(
                MobileAccessRouteErrorKind::BadGateway,
                "invalid control plane response",
            )
        })
}

pub(super) async fn revoke_control_plane_mobile_access_best_effort(managed_tunnel_grant: &str) {
    let control_plane_url = resolve_control_plane_url();
    if control_plane_url.trim().is_empty() {
        return;
    }
    let binding = match managed_tunnel_binding(managed_tunnel_grant) {
        Ok(binding) => binding,
        Err(err) => {
            tracing::warn!("mobile tunnel revoke skipped because binding is unavailable: {err:?}");
            return;
        }
    };

    let _ = reqwest::Client::new()
        .post(format!(
            "{}/v1/mobile/revoke",
            control_plane_url.trim_end_matches('/')
        ))
        .bearer_auth(managed_tunnel_grant.trim())
        .header("x-ctx-daemon-id", binding.daemon_id)
        .header("x-ctx-device-id", binding.device_id)
        .send()
        .await;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedTunnelBinding {
    daemon_id: String,
    device_id: String,
}

fn managed_tunnel_binding(
    managed_tunnel_grant: &str,
) -> Result<ManagedTunnelBinding, MobileAccessRouteError> {
    let trimmed = managed_tunnel_grant.trim();
    if !trimmed.starts_with("ctmt_") {
        return Err(MobileAccessRouteError::new(
            MobileAccessRouteErrorKind::BadRequest,
            "managed_tunnel_grant must start with ctmt_",
        ));
    }
    let digest = hex::encode(Sha256::digest(trimmed.as_bytes()));
    Ok(ManagedTunnelBinding {
        daemon_id: format!("ctmt-daemon-{digest}"),
        device_id: format!("ctmt-device-{digest}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_tunnel_binding_is_derived_from_grant_without_env() {
        let binding = managed_tunnel_binding("ctmt_local_mobile_tunnel_grant").unwrap();
        assert!(binding.daemon_id.starts_with("ctmt-daemon-"));
        assert!(binding.device_id.starts_with("ctmt-device-"));
        assert_eq!(
            binding.daemon_id.trim_start_matches("ctmt-daemon-"),
            binding.device_id.trim_start_matches("ctmt-device-")
        );
    }

    #[test]
    fn managed_tunnel_binding_rejects_non_grant_tokens() {
        let error = managed_tunnel_binding("not-a-grant").unwrap_err();
        assert_eq!(error.kind(), MobileAccessRouteErrorKind::BadRequest);
    }
}
