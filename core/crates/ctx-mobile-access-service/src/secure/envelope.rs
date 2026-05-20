use ctx_core::ids::MobileDeviceId;
use ctx_store::Store;
use ctx_transport_runtime::mobile_e2ee;

use crate::{
    load_mobile_auth_context_for_profile, MobileAccessServiceError, MobileDeviceSequenceAdvance,
    MobileSecureEnvelope, MobileSecureEnvelopeForRoute, MobileSecureProxyPayload,
    MobileSecureProxyResponsePayload, MobileSecureResponseEncryption,
    OpenMobileSecureRequestResult,
};

pub async fn open_mobile_secure_request(
    store: &Store,
    request: MobileSecureEnvelopeForRoute,
) -> Result<OpenMobileSecureRequestResult, MobileAccessServiceError> {
    let device_uuid = uuid::Uuid::parse_str(request.device_id.trim())
        .map_err(|_| MobileAccessServiceError::bad_request("device_id must be a UUID"))?;
    let cfg = store
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            MobileAccessServiceError::internal("mobile access not configured")
        })?
        .ok_or_else(|| MobileAccessServiceError::bad_request("mobile access not enabled"))?;
    if !cfg.enabled {
        return Err(MobileAccessServiceError::bad_request(
            "mobile access not enabled",
        ));
    }

    let device = store
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile device: {e:?}");
            MobileAccessServiceError::internal("failed to read device")
        })?
        .ok_or_else(|| MobileAccessServiceError::unauthorized("unknown device"))?;
    if device.profile_id != cfg.profile_id {
        return Err(MobileAccessServiceError::unauthorized(
            "device not authorized for this tunnel",
        ));
    }

    let Some(device_public_key) = device.public_key.as_ref() else {
        return Err(MobileAccessServiceError::bad_request(
            "device missing public key",
        ));
    };
    let key = mobile_e2ee::derive_key(
        &request.device_id,
        device_public_key,
        &cfg.daemon_private_key,
    )
    .map_err(|_| MobileAccessServiceError::bad_request("failed to derive device key"))?;

    let plaintext = mobile_e2ee::decrypt(
        &key,
        &request.device_id,
        request.seq,
        &request.nonce,
        &request.ciphertext,
    )
    .map_err(|_| MobileAccessServiceError::bad_request("failed to decrypt request"))?;

    let payload: MobileSecureProxyPayload = serde_json::from_slice(&plaintext)
        .map_err(|_| MobileAccessServiceError::bad_request("invalid secure request payload"))?;

    let normalized_path = payload.path.trim();
    if normalized_path.starts_with("/api/mobile/secure")
        || normalized_path.starts_with("/api/mobile/pair")
        || normalized_path.starts_with("/api/mobile/")
    {
        return Err(MobileAccessServiceError::bad_request(
            "secure proxy cannot target mobile management endpoints",
        ));
    }

    match store
        .advance_mobile_device_seq(MobileDeviceId(device_uuid), request.seq)
        .await
        .map_err(|e| {
            tracing::error!("failed to update device seq: {e:?}");
            MobileAccessServiceError::internal("failed to update device")
        })?
        .into()
    {
        MobileDeviceSequenceAdvance::Advanced => {}
        MobileDeviceSequenceAdvance::Stale { current } => {
            tracing::warn!(device_id = %device_uuid, seq = request.seq, current, "rejected stale mobile secure request");
            return Err(MobileAccessServiceError::conflict("stale request sequence"));
        }
        MobileDeviceSequenceAdvance::Missing => {
            return Err(MobileAccessServiceError::not_found("device not registered"));
        }
    }

    let mobile_auth = load_mobile_auth_context_for_profile(store, device.profile_id)
        .await
        .map_err(|_| MobileAccessServiceError::internal("failed to read mobile access profile"))?;

    Ok(OpenMobileSecureRequestResult {
        device_uuid,
        mobile_auth,
        payload,
        response_encryption: MobileSecureResponseEncryption::new(
            request.device_id,
            request.seq,
            key,
        ),
    })
}

pub async fn encrypt_mobile_secure_response(
    context: MobileSecureResponseEncryption,
    response: MobileSecureProxyResponsePayload,
) -> Result<MobileSecureEnvelope, MobileAccessServiceError> {
    let response_bytes = serde_json::to_vec(&response)
        .map_err(|_| MobileAccessServiceError::internal("failed to encode secure response"))?;
    mobile_e2ee::encrypt(
        &context.key,
        &context.device_id,
        context.seq,
        &response_bytes,
    )
    .map(Into::into)
    .map_err(|_| MobileAccessServiceError::internal("failed to encrypt response"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MobileSecureProxyResponsePayload;
    use ctx_transport_runtime::mobile_e2ee;

    #[tokio::test]
    async fn response_encryption_preserves_device_id_and_request_sequence() {
        let device_id = "22222222-2222-2222-2222-222222222222".to_string();
        let (daemon_public_key, daemon_private_key) = mobile_e2ee::generate_keypair();
        let (device_public_key, device_secret_key) = mobile_e2ee::generate_keypair();
        let daemon_key =
            mobile_e2ee::derive_key(&device_id, &device_public_key, &daemon_private_key)
                .expect("daemon key");
        let client_key =
            mobile_e2ee::derive_client_key(&device_id, &device_secret_key, &daemon_public_key)
                .expect("client key");

        let envelope = encrypt_mobile_secure_response(
            MobileSecureResponseEncryption::new(device_id.clone(), 42, daemon_key),
            MobileSecureProxyResponsePayload {
                status: 200,
                headers: Vec::new(),
                body_b64: String::new(),
            },
        )
        .await
        .expect("encrypted response");

        assert_eq!(envelope.device_id, device_id);
        assert_eq!(envelope.seq, 42);
        mobile_e2ee::decrypt(
            &client_key,
            &envelope.device_id,
            envelope.seq,
            &envelope.nonce,
            &envelope.ciphertext,
        )
        .expect("client decrypts response");
    }
}
