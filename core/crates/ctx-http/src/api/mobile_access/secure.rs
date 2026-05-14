use super::secure_proxy::{
    mobile_scope_required_secure_response, proxy_secure_request, SecureProxyError,
    SecureProxyRouterState,
};
use super::*;
use ctx_transport_runtime::mobile_e2ee;
use request::verify_mobile_secure_request;

mod request;

pub(in crate::api) async fn handle_mobile_secure(
    State(state): State<CoreHandle>,
    State(router_state): State<SecureProxyRouterState>,
    body: Bytes,
) -> Result<Json<SecureEnvelope>, (StatusCode, Json<ApiErrorResp>)> {
    let req: MobileSecureEnvelope = parse_json_body(body)?;
    let verified = verify_mobile_secure_request(&state, req).await?;

    match state
        .advance_mobile_device_seq(MobileDeviceId(verified.device_uuid), verified.seq)
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
            tracing::warn!(device_id = %verified.device_uuid, seq = verified.seq, current, "rejected stale mobile secure request");
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

    let response_payload = match state
        .load_mobile_auth_context_for_profile(verified.profile_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to read mobile access profile".into(),
                }),
            )
        })? {
        Some(mobile_auth) if mobile_auth.allows(MobileScope::WorkspaceRead) => {
            proxy_secure_request(&router_state, mobile_auth, verified.payload)
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
    let envelope = mobile_e2ee::encrypt(
        &verified.key,
        &verified.device_id,
        verified.seq,
        &response_bytes,
    )
    .map_err(|_| {
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
