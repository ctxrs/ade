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
    let normalized_base = normalize_profile_base_url(&req.base_url)?;
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
    let qr_payload = build_connection_profile_qr_payload(&profile, &normalized_base, &token);
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

fn normalize_profile_base_url(
    base_url_raw: &str,
) -> Result<String, (StatusCode, Json<ApiErrorResp>)> {
    let base_url_raw = base_url_raw.trim();
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
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn build_connection_profile_qr_payload(
    profile: &MobileConnectionProfile,
    normalized_base: &str,
    token: &str,
) -> serde_json::Value {
    serde_json::json!({
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
    })
}
