use super::*;
use ctx_transport_runtime::mobile_e2ee;

pub(in crate::api) async fn pair_mobile_device(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<Json<SecureEnvelope>, (StatusCode, Json<ApiErrorResp>)> {
    let req: PairMobileDeviceReq = parse_json_body(body)?;
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "mobile access not configured".into(),
                }),
            )
        })?;
    let Some(cfg) = cfg else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "mobile access not enabled".into(),
            }),
        ));
    };
    if !cfg.enabled {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "mobile access not enabled".into(),
            }),
        ));
    }
    let Some(mobile_auth) = load_mobile_auth_context_for_profile(&state, cfg.profile_id)
        .await
        .map_err(|status| {
            (
                status,
                Json(ApiErrorResp {
                    error: "failed to read mobile access profile".into(),
                }),
            )
        })?
    else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: MobileScope::DeviceRegistration.missing_error().into(),
            }),
        ));
    };
    if !mobile_auth.allows(MobileScope::DeviceRegistration) {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: MobileScope::DeviceRegistration.missing_error().into(),
            }),
        ));
    }

    let device_id = req.device_id.trim().to_string();
    let device_public_key = req.public_key.trim().to_string();
    let device_uuid = uuid::Uuid::parse_str(&device_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "device_id must be a UUID".into(),
            }),
        )
    })?;

    let key = mobile_e2ee::derive_key(&device_id, &device_public_key, &cfg.daemon_private_key)
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "failed to derive pairing key".into(),
                }),
            )
        })?;
    if req.seq != 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "pairing seq must be 0".into(),
            }),
        ));
    }
    let decrypted = mobile_e2ee::decrypt_pairing_request(
        &key,
        &device_id,
        &device_public_key,
        &req.nonce,
        &req.ciphertext,
    )
    .map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "failed to decrypt pairing request".into(),
            }),
        )
    })?;
    let payload: PairMobileDevicePayload = serde_json::from_slice(&decrypted).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid pairing request payload".into(),
            }),
        )
    })?;
    let token_hash = hash_pairing_token(payload.pairing_token.trim());
    let allowed = state
        .global_store()
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
        .global_store()
        .upsert_mobile_device(
            MobileDeviceId(device_uuid),
            cfg.profile_id,
            MobileDeviceUpsert {
                device_label: payload
                    .device_label
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                platform: payload
                    .platform
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                push_token: None,
                push_provider: None,
                public_key: Some(device_public_key),
                app_version: payload
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
        "device_id": device_id.clone(),
        "daemon_public_key": cfg.daemon_public_key,
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
    let envelope = mobile_e2ee::encrypt(&key, &device_id, 0, &plaintext).map_err(|_| {
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
