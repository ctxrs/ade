use super::types::RegisterMobileDeviceReq;
use super::*;

pub(in crate::api) async fn list_mobile_devices_for_profile(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<MobileDeviceRegistration>>, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let devices = state
        .global_store()
        .list_mobile_devices(ConnectionProfileId(uuid))
        .await
        .map_err(|e| {
            tracing::error!("failed to list mobile devices: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(devices))
}

pub(in crate::api) async fn register_mobile_device(
    State(state): State<Arc<AppState>>,
    auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<RegisterMobileDeviceReq>,
) -> Result<Json<MobileDeviceRegistration>, (StatusCode, Json<ApiErrorResp>)> {
    let Some(Extension(mobile_auth)) = auth else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "mobile token required".into(),
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
    let device_uuid = uuid::Uuid::parse_str(req.device_id.trim()).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "device_id must be a UUID".into(),
            }),
        )
    })?;
    let device = state
        .global_store()
        .upsert_mobile_device(
            MobileDeviceId(device_uuid),
            mobile_auth.profile_id,
            MobileDeviceUpsert {
                device_label: sanitize_optional_mobile_field(req.device_label),
                platform: sanitize_optional_mobile_field(req.platform),
                push_token: sanitize_optional_mobile_field(req.push_token),
                push_provider: sanitize_optional_mobile_field(req.push_provider),
                public_key: sanitize_optional_mobile_field(req.public_key),
                app_version: sanitize_optional_mobile_field(req.app_version),
            },
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to register mobile device: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to register device".into(),
                }),
            )
        })?;
    Ok(Json(device))
}

fn sanitize_optional_mobile_field(input: Option<String>) -> Option<String> {
    input
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
