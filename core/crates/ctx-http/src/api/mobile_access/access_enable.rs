use super::access::mobile_public_url_is_allowed;
use super::*;
use ctx_transport_runtime::mobile_e2ee;

mod entitlement;
mod pairing;
mod profile_config;
mod response;

use entitlement::{parse_allowed_public_url, request_control_plane_enable};
use pairing::create_mobile_pairing_bootstrap;
use profile_config::{load_or_create_managed_mobile_access_keys, persist_mobile_access_config};
use response::{build_enable_mobile_access_response, start_mobile_tunnel_best_effort};

pub(in crate::api) async fn enable_mobile_access(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<EnableMobileAccessReq>,
) -> Result<Json<EnableMobileAccessResp>, (StatusCode, Json<ApiErrorResp>)> {
    if mobile_auth.is_some() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "desktop auth required".into(),
            }),
        ));
    }
    if state.core.auth_token.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "daemon auth token is not configured; refusing to expose daemon publicly"
                    .into(),
            }),
        ));
    }

    let payload = request_control_plane_enable(&req.supabase_token).await?;
    let public_url = parse_allowed_public_url(&payload.public_base_url)?;
    let now = chrono::Utc::now();
    let keys = load_or_create_managed_mobile_access_keys(&state, &public_url, now).await?;
    persist_mobile_access_config(&state, &payload, &public_url, &keys, now).await?;
    let pairing = create_mobile_pairing_bootstrap(&state, now).await?;
    start_mobile_tunnel_best_effort(&state, &payload, &public_url).await;

    Ok(Json(build_enable_mobile_access_response(
        payload,
        &public_url,
        &keys,
        pairing,
    )))
}
