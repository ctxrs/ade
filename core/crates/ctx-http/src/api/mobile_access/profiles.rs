use super::*;

pub(in crate::api) async fn list_mobile_connection_profiles(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
) -> Result<Json<Vec<MobileConnectionProfile>>, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let profiles = state
        .global_store()
        .list_mobile_connection_profiles()
        .await
        .map_err(|e| {
            tracing::error!("failed to list mobile profiles: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(profiles))
}

pub(in crate::api) async fn create_mobile_connection_profile(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Json(req): Json<CreateMobileConnectionProfileReq>,
) -> Result<Json<CreateMobileConnectionProfileResp>, (StatusCode, Json<ApiErrorResp>)> {
    if mobile_auth.is_some() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "desktop auth required".into(),
            }),
        ));
    }
    let label = req.label.trim();
    if label.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "label is required".into(),
            }),
        ));
    }
    let base_url_raw = req.base_url.trim();
    if base_url_raw.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "base_url is required".into(),
            }),
        ));
    }
    let parsed = Url::parse(base_url_raw).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "base_url must be a valid URL".into(),
            }),
        )
    })?;
    if parsed.scheme() != "https" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "base_url must use https://".into(),
            }),
        ));
    }
    let normalized_base = parsed.as_str().trim_end_matches('/').to_string();
    let scopes = mobile_scope_set_from_strings(&req.scopes)
        .map(|scope_set| scope_set.to_strings())
        .map_err(|error| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error })))?;
    let token = generate_mobile_api_token();
    let token_hash = hash_api_token(&token);
    let token_prefix: String = token.chars().take(8).collect();
    let profile = state
        .global_store()
        .create_mobile_connection_profile(
            label.to_string(),
            normalized_base.clone(),
            token_hash,
            token_prefix,
            scopes,
        )
        .await
        .map_err(|e| {
            tracing::error!("failed to create mobile profile: {e:?}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to create profile".into(),
                }),
            )
        })?;
    let qr_payload = serde_json::json!({
        "connection_profile": {
            "label": profile.label,
            "connection": {
                "type": "direct_https",
                "base_url": normalized_base,
            },
            "auth": {
                "api_token": token,
            }
        },
        "label": profile.label,
        "baseUrl": normalized_base,
        "token": token,
    });
    Ok(Json(CreateMobileConnectionProfileResp {
        profile,
        token,
        qr_payload,
    }))
}

pub(in crate::api) async fn delete_mobile_connection_profile(
    State(state): State<Arc<AppState>>,
    mobile_auth: Option<Extension<MobileAuthContext>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    if mobile_auth.is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    if state
        .global_store()
        .get_mobile_connection_profile(ConnectionProfileId(uuid))
        .await
        .map_err(|e| {
            tracing::error!("failed to load mobile profile before delete: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }
    state
        .global_store()
        .delete_mobile_connection_profile(ConnectionProfileId(uuid))
        .await
        .map_err(|e| {
            tracing::error!("failed to delete mobile profile: {e:?}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(StatusCode::NO_CONTENT)
}

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
    let sanitize = |input: Option<String>| -> Option<String> {
        input
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let device = state
        .global_store()
        .upsert_mobile_device(
            MobileDeviceId(device_uuid),
            mobile_auth.profile_id,
            MobileDeviceUpsert {
                device_label: sanitize(req.device_label),
                platform: sanitize(req.platform),
                push_token: sanitize(req.push_token),
                push_provider: sanitize(req.push_provider),
                public_key: sanitize(req.public_key),
                app_version: sanitize(req.app_version),
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
