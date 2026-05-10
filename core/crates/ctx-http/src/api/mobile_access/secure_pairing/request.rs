use std::sync::Arc;

use axum::http::StatusCode;
use axum::Json;
use ctx_store::store::MobileAccessConfig;
use ctx_transport_runtime::mobile_e2ee::{self, E2eeKey};

use super::super::{ApiErrorResp, MobileScope, PairMobileDevicePayload, PairMobileDeviceReq};
use crate::api::auth::load_mobile_auth_context_for_profile;
use crate::daemon::AppState;

pub(super) struct VerifiedMobilePairingRequest {
    pub(super) device_uuid: uuid::Uuid,
    pub(super) device_id: String,
    pub(super) device_public_key: String,
    pub(super) config: MobileAccessConfig,
    pub(super) key: E2eeKey,
    pub(super) payload: PairMobileDevicePayload,
}

pub(super) async fn verify_mobile_pairing_request(
    state: &Arc<AppState>,
    req: PairMobileDeviceReq,
) -> Result<VerifiedMobilePairingRequest, (StatusCode, Json<ApiErrorResp>)> {
    let config = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|e| {
            tracing::error!("failed to read mobile access config: {e:?}");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "mobile access not configured",
            )
        })?;
    let Some(config) = config else {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "mobile access not enabled",
        ));
    };
    if !config.enabled {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "mobile access not enabled",
        ));
    }

    let Some(mobile_auth) = load_mobile_auth_context_for_profile(state, config.profile_id)
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
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            MobileScope::DeviceRegistration.missing_error(),
        ));
    };
    if !mobile_auth.allows(MobileScope::DeviceRegistration) {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            MobileScope::DeviceRegistration.missing_error(),
        ));
    }

    let device_id = req.device_id.trim().to_string();
    let device_public_key = req.public_key.trim().to_string();
    let device_uuid = uuid::Uuid::parse_str(&device_id)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "device_id must be a UUID"))?;

    let key = mobile_e2ee::derive_key(&device_id, &device_public_key, &config.daemon_private_key)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "failed to derive pairing key"))?;
    if req.seq != 0 {
        return Err(api_error(StatusCode::BAD_REQUEST, "pairing seq must be 0"));
    }

    let decrypted = mobile_e2ee::decrypt_pairing_request(
        &key,
        &device_id,
        &device_public_key,
        &req.nonce,
        &req.ciphertext,
    )
    .map_err(|_| api_error(StatusCode::BAD_REQUEST, "failed to decrypt pairing request"))?;
    let payload: PairMobileDevicePayload = serde_json::from_slice(&decrypted)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid pairing request payload"))?;

    Ok(VerifiedMobilePairingRequest {
        device_uuid,
        device_id,
        device_public_key,
        config,
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
