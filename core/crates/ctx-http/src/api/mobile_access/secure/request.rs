use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::{ConnectionProfileId, MobileDeviceId};
use ctx_transport_runtime::mobile_e2ee::{self, E2eeKey};

use super::super::{ApiErrorResp, MobileSecureEnvelope, SecureRequestPayload};
use crate::daemon::CoreHandle;

pub(super) struct VerifiedMobileSecureRequest {
    pub(super) device_uuid: uuid::Uuid,
    pub(super) device_id: String,
    pub(super) seq: i64,
    pub(super) profile_id: ConnectionProfileId,
    pub(super) key: E2eeKey,
    pub(super) payload: SecureRequestPayload,
}

pub(super) async fn verify_mobile_secure_request(
    state: &CoreHandle,
    req: MobileSecureEnvelope,
) -> Result<VerifiedMobileSecureRequest, (StatusCode, Json<ApiErrorResp>)> {
    let device_uuid = uuid::Uuid::parse_str(req.device_id.trim())
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "device_id must be a UUID"))?;
    let cfg = state.get_mobile_access_config().await.map_err(|e| {
        tracing::error!("failed to read mobile access config: {e:?}");
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "mobile access not configured",
        )
    })?;
    let Some(cfg) = cfg else {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "mobile access not enabled",
        ));
    };
    if !cfg.enabled {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "mobile access not enabled",
        ));
    }
    let device = state
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile device: {e:?}");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "failed to read device")
        })?
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "unknown device"))?;
    if device.profile_id != cfg.profile_id {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "device not authorized for this tunnel",
        ));
    }

    let Some(device_public_key) = device.public_key.as_ref() else {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "device missing public key",
        ));
    };
    let key = mobile_e2ee::derive_key(&req.device_id, device_public_key, &cfg.daemon_private_key)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "failed to derive device key"))?;

    let plaintext =
        mobile_e2ee::decrypt(&key, &req.device_id, req.seq, &req.nonce, &req.ciphertext)
            .map_err(|_| api_error(StatusCode::BAD_REQUEST, "failed to decrypt request"))?;

    let payload: SecureRequestPayload = serde_json::from_slice(&plaintext)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid secure request payload"))?;

    let normalized_path = payload.path.trim();
    if normalized_path.starts_with("/api/mobile/secure")
        || normalized_path.starts_with("/api/mobile/pair")
        || normalized_path.starts_with("/api/mobile/")
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "secure proxy cannot target mobile management endpoints",
        ));
    }

    Ok(VerifiedMobileSecureRequest {
        device_uuid,
        device_id: req.device_id,
        seq: req.seq,
        profile_id: device.profile_id,
        key,
        payload,
    })
}

fn api_error(status: StatusCode, error: &'static str) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}
