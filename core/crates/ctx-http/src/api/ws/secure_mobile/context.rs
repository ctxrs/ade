use std::sync::Arc;

use ctx_core::ids::MobileDeviceId;
use ctx_core::models::WorkspaceActiveSnapshotClientMessage;
use ctx_transport_runtime::mobile_e2ee::{self, E2eeKey};

use super::super::super::{
    load_mobile_auth_context_for_profile, MobileScope, MobileSecureEnvelope,
};
use crate::daemon::AppState;

pub(super) struct MobileSecureStreamContext {
    pub(super) device_id: String,
    pub(super) key: E2eeKey,
}

pub(super) async fn load_mobile_secure_stream_context(
    state: &Arc<AppState>,
    device_id: String,
) -> Result<MobileSecureStreamContext, anyhow::Error> {
    let device_uuid = uuid::Uuid::parse_str(&device_id)?;
    let config = state.global_store().get_mobile_access_config().await?;
    let config = config.ok_or_else(|| anyhow::anyhow!("mobile access not configured"))?;
    if !config.enabled {
        return Err(anyhow::anyhow!("mobile access not enabled"));
    }

    let Some(mobile_auth) = load_mobile_auth_context_for_profile(state, config.profile_id)
        .await
        .map_err(|status| anyhow::anyhow!("failed to load mobile access profile: {status}"))?
    else {
        return Err(anyhow::anyhow!(
            "{}",
            MobileScope::WorkspaceStream.missing_error()
        ));
    };
    if !mobile_auth.allows(MobileScope::WorkspaceStream) {
        return Err(anyhow::anyhow!(
            "{}",
            MobileScope::WorkspaceStream.missing_error()
        ));
    }

    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await?
        .ok_or_else(|| anyhow::anyhow!("device not registered"))?;
    if device.profile_id != config.profile_id {
        return Err(anyhow::anyhow!("device not authorized for tunnel"));
    }
    let device_public_key = device
        .public_key
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("device missing public key"))?;
    let key = mobile_e2ee::derive_key(&device_id, device_public_key, &config.daemon_private_key)?;

    Ok(MobileSecureStreamContext { device_id, key })
}

pub(super) fn decode_mobile_secure_client_message(
    context: &MobileSecureStreamContext,
    text: &str,
) -> Result<Option<WorkspaceActiveSnapshotClientMessage>, anyhow::Error> {
    let frame: MobileSecureEnvelope = match serde_json::from_str(text) {
        Ok(frame) => frame,
        Err(_) => return Ok(None),
    };
    let payload = mobile_e2ee::decrypt(
        &context.key,
        &context.device_id,
        frame.seq,
        &frame.nonce,
        &frame.ciphertext,
    )?;
    let message = serde_json::from_slice(&payload)?;
    Ok(Some(message))
}
