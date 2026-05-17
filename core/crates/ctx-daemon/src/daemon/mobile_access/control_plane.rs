use serde::Deserialize;

use super::{MobileAccessRouteError, MobileAccessRouteErrorKind};

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
    supabase_token: &str,
) -> Result<ControlPlaneEnableResp, MobileAccessRouteError> {
    let control_plane_url = resolve_control_plane_url();
    if control_plane_url.trim().is_empty() {
        return Err(MobileAccessRouteError::new(
            MobileAccessRouteErrorKind::BadRequest,
            "CTX_TUNNEL_CONTROL_PLANE_URL is not set",
        ));
    }

    let enable_resp = reqwest::Client::new()
        .post(format!(
            "{}/v1/mobile/enable",
            control_plane_url.trim_end_matches('/')
        ))
        .bearer_auth(supabase_token.trim())
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

pub(super) async fn revoke_control_plane_mobile_access_best_effort(supabase_token: &str) {
    let control_plane_url = resolve_control_plane_url();
    if control_plane_url.trim().is_empty() {
        return;
    }

    let _ = reqwest::Client::new()
        .post(format!(
            "{}/v1/mobile/revoke",
            control_plane_url.trim_end_matches('/')
        ))
        .bearer_auth(supabase_token.trim())
        .send()
        .await;
}
