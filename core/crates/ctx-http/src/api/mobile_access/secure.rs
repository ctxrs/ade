use super::secure_proxy::{
    mobile_scope_required_secure_response, proxy_secure_request, SecureProxyError,
};
use super::*;
use ctx_transport_runtime::mobile_e2ee;

pub(in crate::api) async fn handle_mobile_secure(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<Json<SecureEnvelope>, (StatusCode, Json<ApiErrorResp>)> {
    let req: MobileSecureEnvelope = parse_json_body(body)?;
    let device_uuid = uuid::Uuid::parse_str(req.device_id.trim()).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "device_id must be a UUID".into(),
            }),
        )
    })?;
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
    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile device: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to read device".into(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResp {
                    error: "unknown device".into(),
                }),
            )
        })?;
    if device.profile_id != cfg.profile_id {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "device not authorized for this tunnel".into(),
            }),
        ));
    }

    let Some(device_public_key) = device.public_key.as_ref() else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "device missing public key".into(),
            }),
        ));
    };
    let key = mobile_e2ee::derive_key(&req.device_id, device_public_key, &cfg.daemon_private_key)
        .map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "failed to derive device key".into(),
            }),
        )
    })?;

    let plaintext =
        mobile_e2ee::decrypt(&key, &req.device_id, req.seq, &req.nonce, &req.ciphertext).map_err(
            |_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "failed to decrypt request".into(),
                    }),
                )
            },
        )?;

    let payload: SecureRequestPayload = serde_json::from_slice(&plaintext).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid secure request payload".into(),
            }),
        )
    })?;

    let normalized_path = payload.path.trim();
    if normalized_path.starts_with("/api/mobile/secure")
        || normalized_path.starts_with("/api/mobile/pair")
        || normalized_path.starts_with("/api/mobile/")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "secure proxy cannot target mobile management endpoints".into(),
            }),
        ));
    }

    match state
        .global_store()
        .advance_mobile_device_seq(MobileDeviceId(device_uuid), req.seq)
        .await
        .map_err(|e| {
            tracing::error!("failed to update device seq: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to update device".into(),
                }),
            )
        })? {
        ctx_store::store::MobileDeviceSeqAdvance::Advanced => {}
        ctx_store::store::MobileDeviceSeqAdvance::Stale { current } => {
            tracing::warn!(device_id = %device_uuid, seq = req.seq, current, "rejected stale mobile secure request");
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "stale request sequence".into(),
                }),
            ));
        }
        ctx_store::store::MobileDeviceSeqAdvance::Missing => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "device not registered".into(),
                }),
            ));
        }
    }

    let response_payload = match load_mobile_auth_context_for_profile(&state, device.profile_id)
        .await
        .map_err(|status| {
            (
                status,
                Json(ApiErrorResp {
                    error: "failed to read mobile access profile".into(),
                }),
            )
        })? {
        Some(mobile_auth) if mobile_auth.allows(MobileScope::WorkspaceRead) => {
            proxy_secure_request(&state, mobile_auth, payload)
                .await
                .map_err(SecureProxyError::into_api_error)?
        }
        _ => mobile_scope_required_secure_response(MobileScope::WorkspaceRead)
            .map_err(SecureProxyError::into_api_error)?,
    };

    let response_bytes = serde_json::to_vec(&response_payload).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "failed to encode secure response".into(),
            }),
        )
    })?;
    let envelope =
        mobile_e2ee::encrypt(&key, &req.device_id, req.seq, &response_bytes).map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to encrypt response".into(),
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
