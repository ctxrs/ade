use chrono::Utc;
use ctx_core::ids::MobileDeviceId;
use ctx_store::Store;
use ctx_transport_runtime::mobile_e2ee;

use crate::{
    hash_pairing_token, load_mobile_auth_context_for_profile, MobileAccessConfigSnapshot,
    MobileAccessServiceError, MobileDeviceRegistrationUpdate, MobileScope, MobileSecureEnvelope,
    PairMobileDevicePayload, PairMobileDeviceRequest,
};

pub async fn pair_mobile_device(
    store: &Store,
    request: PairMobileDeviceRequest,
) -> Result<MobileSecureEnvelope, MobileAccessServiceError> {
    let verified = verify_mobile_pairing_request(store, request).await?;
    let token_hash = hash_pairing_token(verified.payload.pairing_token.trim());
    let allowed = store
        .consume_mobile_pairing_token(&token_hash)
        .await
        .map_err(|e| {
            tracing::error!("failed to check pairing token: {e:?}");
            MobileAccessServiceError::internal("failed to validate pairing token")
        })?;
    if !allowed {
        return Err(MobileAccessServiceError::unauthorized(
            "pairing token invalid or expired",
        ));
    }
    let _device = store
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
            MobileAccessServiceError::internal("failed to register device")
        })?;

    let payload = serde_json::json!({
        "paired": true,
        "device_id": verified.device_id.clone(),
        "daemon_public_key": verified.config.daemon_public_key,
        "paired_at": Utc::now().to_rfc3339(),
    });
    let plaintext = serde_json::to_vec(&payload)
        .map_err(|_| MobileAccessServiceError::internal("failed to encode pairing response"))?;
    mobile_e2ee::encrypt(&verified.key, &verified.device_id, 0, &plaintext)
        .map(Into::into)
        .map_err(|_| MobileAccessServiceError::internal("failed to encrypt pairing response"))
}

struct VerifiedMobilePairingRequest {
    device_uuid: uuid::Uuid,
    device_id: String,
    device_public_key: String,
    config: MobileAccessConfigSnapshot,
    key: mobile_e2ee::E2eeKey,
    payload: PairMobileDevicePayload,
}

async fn verify_mobile_pairing_request(
    store: &Store,
    request: PairMobileDeviceRequest,
) -> Result<VerifiedMobilePairingRequest, MobileAccessServiceError> {
    let config = store
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            MobileAccessServiceError::internal("mobile access not configured")
        })?
        .map(MobileAccessConfigSnapshot::from)
        .ok_or_else(|| MobileAccessServiceError::bad_request("mobile access not enabled"))?;
    if !config.enabled {
        return Err(MobileAccessServiceError::bad_request(
            "mobile access not enabled",
        ));
    }

    let Some(mobile_auth) = load_mobile_auth_context_for_profile(store, config.profile_id)
        .await
        .map_err(|_| MobileAccessServiceError::internal("failed to read mobile access profile"))?
    else {
        return Err(MobileAccessServiceError::unauthorized(
            MobileScope::DeviceRegistration.missing_error(),
        ));
    };
    if !mobile_auth.allows(MobileScope::DeviceRegistration) {
        return Err(MobileAccessServiceError::unauthorized(
            MobileScope::DeviceRegistration.missing_error(),
        ));
    }

    let device_id = request.device_id.trim().to_string();
    let device_public_key = request.public_key.trim().to_string();
    let device_uuid = uuid::Uuid::parse_str(&device_id)
        .map_err(|_| MobileAccessServiceError::bad_request("device_id must be a UUID"))?;

    let key = mobile_e2ee::derive_key(&device_id, &device_public_key, &config.daemon_private_key)
        .map_err(|_| MobileAccessServiceError::bad_request("failed to derive pairing key"))?;
    if request.seq != 0 {
        return Err(MobileAccessServiceError::bad_request(
            "pairing seq must be 0",
        ));
    }

    let decrypted = mobile_e2ee::decrypt_pairing_request(
        &key,
        &device_id,
        &device_public_key,
        &request.nonce,
        &request.ciphertext,
    )
    .map_err(|_| MobileAccessServiceError::bad_request("failed to decrypt pairing request"))?;
    let payload: PairMobileDevicePayload = serde_json::from_slice(&decrypted)
        .map_err(|_| MobileAccessServiceError::bad_request("invalid pairing request payload"))?;

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
