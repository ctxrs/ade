use std::sync::Arc;

use ctx_core::ids::MobileDeviceId;
use ctx_transport_runtime::mobile_e2ee::{self, E2eeKey};
use serde::{Deserialize, Serialize};

use super::{
    load_mobile_auth_context_for_profile, MobileAccessRouteError, MobileDeviceSequenceAdvance,
    MobileSecureEnvelope,
};
use crate::daemon::DaemonState;

#[derive(Debug, Clone, Deserialize)]
pub struct MobileSecureEnvelopeForRoute {
    pub device_id: String,
    pub seq: i64,
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MobileSecureProxyPayload {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body_b64: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MobileSecureProxyResponsePayload {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body_b64: String,
}

#[derive(Debug, Clone)]
pub struct MobileSecureResponseEncryption {
    pub device_id: String,
    pub seq: i64,
    key: E2eeKey,
}

#[derive(Debug, Clone)]
pub struct OpenMobileSecureRequestResult {
    pub device_uuid: uuid::Uuid,
    pub mobile_auth: Option<super::MobileAuthContext>,
    pub payload: MobileSecureProxyPayload,
    pub response_encryption: MobileSecureResponseEncryption,
}

pub(super) async fn open_mobile_secure_request_for_route(
    state: &Arc<DaemonState>,
    request: MobileSecureEnvelopeForRoute,
) -> Result<OpenMobileSecureRequestResult, MobileAccessRouteError> {
    let device_uuid = uuid::Uuid::parse_str(request.device_id.trim())
        .map_err(|_| MobileAccessRouteError::bad_request("device_id must be a UUID"))?;
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            MobileAccessRouteError::internal("mobile access not configured")
        })?;
    let Some(cfg) = cfg else {
        return Err(MobileAccessRouteError::bad_request(
            "mobile access not enabled",
        ));
    };
    if !cfg.enabled {
        return Err(MobileAccessRouteError::bad_request(
            "mobile access not enabled",
        ));
    }
    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile device: {e:?}");
            MobileAccessRouteError::internal("failed to read device")
        })?
        .ok_or_else(|| MobileAccessRouteError::unauthorized("unknown device"))?;
    if device.profile_id != cfg.profile_id {
        return Err(MobileAccessRouteError::unauthorized(
            "device not authorized for this tunnel",
        ));
    }

    let Some(device_public_key) = device.public_key.as_ref() else {
        return Err(MobileAccessRouteError::bad_request(
            "device missing public key",
        ));
    };
    let key = mobile_e2ee::derive_key(
        &request.device_id,
        device_public_key,
        &cfg.daemon_private_key,
    )
    .map_err(|_| MobileAccessRouteError::bad_request("failed to derive device key"))?;

    let plaintext = mobile_e2ee::decrypt(
        &key,
        &request.device_id,
        request.seq,
        &request.nonce,
        &request.ciphertext,
    )
    .map_err(|_| MobileAccessRouteError::bad_request("failed to decrypt request"))?;

    let payload: MobileSecureProxyPayload = serde_json::from_slice(&plaintext)
        .map_err(|_| MobileAccessRouteError::bad_request("invalid secure request payload"))?;

    let normalized_path = payload.path.trim();
    if normalized_path.starts_with("/api/mobile/secure")
        || normalized_path.starts_with("/api/mobile/pair")
        || normalized_path.starts_with("/api/mobile/")
    {
        return Err(MobileAccessRouteError::bad_request(
            "secure proxy cannot target mobile management endpoints",
        ));
    }

    match state
        .global_store()
        .advance_mobile_device_seq(MobileDeviceId(device_uuid), request.seq)
        .await
        .map_err(|e| {
            tracing::error!("failed to update device seq: {e:?}");
            MobileAccessRouteError::internal("failed to update device")
        })?
        .into()
    {
        MobileDeviceSequenceAdvance::Advanced => {}
        MobileDeviceSequenceAdvance::Stale { current } => {
            tracing::warn!(device_id = %device_uuid, seq = request.seq, current, "rejected stale mobile secure request");
            return Err(MobileAccessRouteError::new(
                super::MobileAccessRouteErrorKind::Conflict,
                "stale request sequence",
            ));
        }
        MobileDeviceSequenceAdvance::Missing => {
            return Err(MobileAccessRouteError::new(
                super::MobileAccessRouteErrorKind::NotFound,
                "device not registered",
            ));
        }
    }

    let mobile_auth = load_mobile_auth_context_for_profile(state, device.profile_id)
        .await
        .map_err(|_| MobileAccessRouteError::internal("failed to read mobile access profile"))?;

    Ok(OpenMobileSecureRequestResult {
        device_uuid,
        mobile_auth,
        payload,
        response_encryption: MobileSecureResponseEncryption {
            device_id: request.device_id,
            seq: request.seq,
            key,
        },
    })
}

pub(super) async fn encrypt_mobile_secure_response_for_route(
    context: MobileSecureResponseEncryption,
    response: MobileSecureProxyResponsePayload,
) -> Result<MobileSecureEnvelope, MobileAccessRouteError> {
    let response_bytes = serde_json::to_vec(&response)
        .map_err(|_| MobileAccessRouteError::internal("failed to encode secure response"))?;
    mobile_e2ee::encrypt(
        &context.key,
        &context.device_id,
        context.seq,
        &response_bytes,
    )
    .map(Into::into)
    .map_err(|_| MobileAccessRouteError::internal("failed to encrypt response"))
}
