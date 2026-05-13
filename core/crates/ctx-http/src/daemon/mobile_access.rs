use std::sync::Arc;

use ctx_core::ids::ConnectionProfileId;
use ctx_transport_runtime::mobile_tunnel::{MobileTunnelState, StartMobileTunnelConfig};

use crate::daemon::AppState;

#[derive(Debug, Clone)]
pub(crate) struct MobileAccessStatusSnapshot {
    pub(crate) enabled: bool,
    pub(crate) tunnel_id: Option<String>,
    pub(crate) public_base_url: Option<String>,
    pub(crate) relay_base_url: Option<String>,
    pub(crate) daemon_public_key: Option<String>,
    pub(crate) tunnel_state: MobileTunnelState,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StartMobileTunnelRequest {
    pub(crate) relay_base_url: String,
    pub(crate) tunnel_id: String,
    pub(crate) tunnel_secret: String,
    pub(crate) public_base_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MobileAccessStatusError {
    ReadConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DisableMobileAccessError {
    ReadConfig,
    ClearPairingTokens,
    DeleteConfig,
    DeleteConnectionProfile,
}

pub(crate) async fn mobile_access_status(
    state: &Arc<AppState>,
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

pub(crate) async fn start_mobile_tunnel_best_effort(
    state: &Arc<AppState>,
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

pub(crate) async fn disable_mobile_access_runtime(
    state: &Arc<AppState>,
) -> Result<(), DisableMobileAccessError> {
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|err| {
            tracing::error!(
                "failed to read mobile access config while disabling mobile access: {err:?}"
            );
            DisableMobileAccessError::ReadConfig
        })?;

    state.transport.mobile_tunnel.stop().await;
    state
        .global_store()
        .clear_mobile_pairing_tokens()
        .await
        .map_err(|err| {
            tracing::error!(
                "failed to clear pairing tokens while disabling mobile access: {err:?}"
            );
            DisableMobileAccessError::ClearPairingTokens
        })?;

    if let Some(cfg) = cfg {
        delete_mobile_access_config(state, cfg.profile_id).await?;
    }
    Ok(())
}

async fn delete_mobile_access_config(
    state: &Arc<AppState>,
    profile_id: ConnectionProfileId,
) -> Result<(), DisableMobileAccessError> {
    state
        .global_store()
        .delete_mobile_access_config()
        .await
        .map_err(|err| {
            tracing::error!(
                "failed to delete mobile access config while disabling mobile access: {err:?}"
            );
            DisableMobileAccessError::DeleteConfig
        })?;
    state
        .global_store()
        .delete_mobile_connection_profile(profile_id)
        .await
        .map_err(|err| {
            tracing::error!(
                "failed to delete mobile connection profile while disabling mobile access: {err:?}"
            );
            DisableMobileAccessError::DeleteConnectionProfile
        })?;
    Ok(())
}
