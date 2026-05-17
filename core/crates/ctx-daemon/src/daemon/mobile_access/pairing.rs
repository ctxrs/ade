use std::sync::Arc;

use chrono::Utc;
use ctx_core::ids::MobileDeviceId;
use ctx_transport_runtime::mobile_e2ee;

use super::tokens::hash_pairing_token;
use super::{
    load_mobile_auth_context_for_profile, MobileAccessRouteError, MobileAccessRouteErrorKind,
    MobileDeviceRegistrationUpdate, MobileScope, MobileSecureEnvelope, PairMobileDevicePayload,
    PairMobileDeviceRequest,
};
use crate::daemon::DaemonState;

pub(super) async fn pair_mobile_device_for_route(
    state: &Arc<DaemonState>,
    request: PairMobileDeviceRequest,
) -> Result<MobileSecureEnvelope, MobileAccessRouteError> {
    let verified = verify_mobile_pairing_request(state, request).await?;
    let token_hash = hash_pairing_token(verified.payload.pairing_token.trim());
    let allowed = state
        .global_store()
        .consume_mobile_pairing_token(&token_hash)
        .await
        .map_err(|e| {
            tracing::error!("failed to check pairing token: {e:?}");
            MobileAccessRouteError::internal("failed to validate pairing token")
        })?;
    if !allowed {
        return Err(MobileAccessRouteError::unauthorized(
            "pairing token invalid or expired",
        ));
    }
    let _device = state
        .global_store()
        .upsert_mobile_device(
            MobileDeviceId(verified.device_uuid),
            verified.config.profile_id,
            MobileDeviceRegistrationUpdate {
                device_label: sanitize_optional_mobile_field(verified.payload.device_label),
                platform: sanitize_optional_mobile_field(verified.payload.platform),
                push_token: None,
                push_provider: None,
                public_key: Some(verified.device_public_key.clone()),
                app_version: sanitize_optional_mobile_field(verified.payload.app_version),
            }
            .into(),
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to register device: {e:?}");
            MobileAccessRouteError::internal("failed to register device")
        })?;

    let payload = serde_json::json!({
        "paired": true,
        "device_id": verified.device_id.clone(),
        "daemon_public_key": verified.config.daemon_public_key,
        "paired_at": Utc::now().to_rfc3339(),
    });
    let plaintext = serde_json::to_vec(&payload)
        .map_err(|_| MobileAccessRouteError::internal("failed to encode pairing response"))?;
    mobile_e2ee::encrypt(&verified.key, &verified.device_id, 0, &plaintext)
        .map(Into::into)
        .map_err(|_| MobileAccessRouteError::internal("failed to encrypt pairing response"))
}

struct VerifiedMobilePairingRequest {
    device_uuid: uuid::Uuid,
    device_id: String,
    device_public_key: String,
    config: super::MobileAccessConfigSnapshot,
    key: mobile_e2ee::E2eeKey,
    payload: PairMobileDevicePayload,
}

async fn verify_mobile_pairing_request(
    state: &Arc<DaemonState>,
    request: PairMobileDeviceRequest,
) -> Result<VerifiedMobilePairingRequest, MobileAccessRouteError> {
    let config = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            MobileAccessRouteError::new(
                MobileAccessRouteErrorKind::Internal,
                "mobile access not configured",
            )
        })?;
    let Some(config) = config.map(super::MobileAccessConfigSnapshot::from) else {
        return Err(MobileAccessRouteError::bad_request(
            "mobile access not enabled",
        ));
    };
    if !config.enabled {
        return Err(MobileAccessRouteError::bad_request(
            "mobile access not enabled",
        ));
    }

    let Some(mobile_auth) = load_mobile_auth_context_for_profile(state, config.profile_id)
        .await
        .map_err(|_| MobileAccessRouteError::internal("failed to read mobile access profile"))?
    else {
        return Err(MobileAccessRouteError::unauthorized(
            MobileScope::DeviceRegistration.missing_error(),
        ));
    };
    if !mobile_auth.allows(MobileScope::DeviceRegistration) {
        return Err(MobileAccessRouteError::unauthorized(
            MobileScope::DeviceRegistration.missing_error(),
        ));
    }

    let device_id = request.device_id.trim().to_string();
    let device_public_key = request.public_key.trim().to_string();
    let device_uuid = uuid::Uuid::parse_str(&device_id)
        .map_err(|_| MobileAccessRouteError::bad_request("device_id must be a UUID"))?;

    let key = mobile_e2ee::derive_key(&device_id, &device_public_key, &config.daemon_private_key)
        .map_err(|_| MobileAccessRouteError::bad_request("failed to derive pairing key"))?;
    if request.seq != 0 {
        return Err(MobileAccessRouteError::bad_request("pairing seq must be 0"));
    }

    let decrypted = mobile_e2ee::decrypt_pairing_request(
        &key,
        &device_id,
        &device_public_key,
        &request.nonce,
        &request.ciphertext,
    )
    .map_err(|_| MobileAccessRouteError::bad_request("failed to decrypt pairing request"))?;
    let payload: PairMobileDevicePayload = serde_json::from_slice(&decrypted)
        .map_err(|_| MobileAccessRouteError::bad_request("invalid pairing request payload"))?;

    Ok(VerifiedMobilePairingRequest {
        device_uuid,
        device_id,
        device_public_key,
        config,
        key,
        payload,
    })
}

fn sanitize_optional_mobile_field(input: Option<String>) -> Option<String> {
    input
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
