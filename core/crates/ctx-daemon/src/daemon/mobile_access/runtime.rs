use std::sync::Arc;

use ctx_mobile_access_service::{
    finish_mobile_access_disable_cleanup, persist_mobile_access_disabled_state,
    route_contract::{DisableMobileAccessError, MobileAccessStatusSnapshot},
};
use ctx_transport_runtime::mobile_tunnel::StartMobileTunnelConfig;

use crate::daemon::DaemonState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartMobileTunnelRequest {
    pub relay_base_url: String,
    pub tunnel_id: String,
    pub tunnel_secret: String,
    pub public_base_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileAccessStatusError {
    ReadConfig,
}

pub async fn mobile_access_status(
    state: &Arc<DaemonState>,
) -> Result<MobileAccessStatusSnapshot, MobileAccessStatusError> {
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|err| {
            tracing::error!("failed to read mobile access config: {err:?}");
            MobileAccessStatusError::ReadConfig
        })?;
    let tunnel_status = state.transport.mobile_tunnel.status().await;
    let (enabled, tunnel_id, public_base_url, relay_base_url, daemon_public_key) = match cfg {
        Some(cfg) => (
            cfg.enabled,
            Some(cfg.tunnel_id),
            Some(cfg.public_base_url),
            Some(cfg.relay_base_url),
            Some(cfg.daemon_public_key),
        ),
        None => (false, None, None, None, None),
    };
    Ok(MobileAccessStatusSnapshot {
        enabled,
        tunnel_id,
        public_base_url,
        relay_base_url,
        daemon_public_key,
        tunnel_state: tunnel_status.state,
        last_error: tunnel_status.last_error,
    })
}

pub async fn start_mobile_tunnel_best_effort(
    state: &Arc<DaemonState>,
    request: StartMobileTunnelRequest,
) {
    let tunnel_cfg = StartMobileTunnelConfig {
        relay_base_url: request.relay_base_url,
        tunnel_id: request.tunnel_id,
        tunnel_secret: request.tunnel_secret,
        public_base_url: request.public_base_url.trim_end_matches('/').to_string(),
        local_daemon_url: state.core.daemon_url.trim_end_matches('/').to_string(),
    };
    if let Err(err) = state.transport.mobile_tunnel.start(tunnel_cfg).await {
        tracing::warn!("failed to start mobile tunnel: {err:#}");
    }
}

pub async fn disable_mobile_access_runtime(
    state: &Arc<DaemonState>,
) -> Result<(), DisableMobileAccessError> {
    let disabled = persist_mobile_access_disabled_state(state.global_store())
        .await
        .map_err(DisableMobileAccessError::from)?;
    state.transport.mobile_tunnel.stop().await;
    finish_mobile_access_disable_cleanup(state.global_store(), disabled)
        .await
        .map_err(DisableMobileAccessError::from)
}
