use super::*;
use ctx_transport_runtime::mobile_e2ee;
use request::verify_mobile_pairing_request;

mod request;

pub(in crate::api) async fn pair_mobile_device(
    State(state): State<CoreHandle>,
    body: Bytes,
) -> Result<Json<SecureEnvelope>, (StatusCode, Json<ApiErrorResp>)> {
    let req: PairMobileDeviceReq = parse_json_body(body)?;
    let verified = verify_mobile_pairing_request(&state, req).await?;
    let token_hash = hash_pairing_token(verified.payload.pairing_token.trim());
    let allowed = state
        .consume_mobile_pairing_token(&token_hash)
        .await
        .map_err(|e| {
            tracing::error!("failed to check pairing token: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to validate pairing token".into(),
                }),
            )
        })?;
    if !allowed {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "pairing token invalid or expired".into(),
            }),
        ));
    }
    let _device = state
        .upsert_mobile_device(
            MobileDeviceId(verified.device_uuid),
            verified.config.profile_id,
            MobileDeviceRegistrationUpdate {
                device_label: verified
                    .payload
                    .device_label
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                platform: verified
                    .payload
                    .platform
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                push_token: None,
                push_provider: None,
                public_key: Some(verified.device_public_key),
                app_version: verified
                    .payload
                    .app_version
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
            },
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to register device: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to register device".into(),
                }),
            )
        })?;

    let payload = serde_json::json!({
        "paired": true,
        "device_id": verified.device_id.clone(),
        "daemon_public_key": verified.config.daemon_public_key,
        "paired_at": chrono::Utc::now().to_rfc3339(),
    });
    let plaintext = serde_json::to_vec(&payload).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "failed to encode pairing response".into(),
            }),
        )
    })?;
    let envelope = mobile_e2ee::encrypt(&verified.key, &verified.device_id, 0, &plaintext)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to encrypt pairing response".into(),
                }),
            )
        })?;

    Ok(Json(SecureEnvelope {
        device_id: envelope.device_id,
        seq: envelope.seq,
        nonce: envelope.nonce_b64,
        ciphertext: envelope.ciphertext_b64,
    }))
}
