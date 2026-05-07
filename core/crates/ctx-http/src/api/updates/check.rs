use super::*;

#[derive(Debug, Serialize)]
pub(in crate::api) struct UpdateCheckResp {
    channel: String,
    base_url: String,
    platform: Option<String>,
    current_version: String,
    latest_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_supported_version: Option<String>,
    platform_supported: bool,
    in_place_update_supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    in_place_update_reason: Option<String>,
    update_available: bool,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    manifest: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct UpdateCheckQuery {
    #[serde(default)]
    channel: Option<String>,
}

pub(in crate::api) async fn check_updates(
    State(_state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<UpdateCheckQuery>,
) -> Result<Json<UpdateCheckResp>, (StatusCode, Json<ApiErrorResp>)> {
    let channel =
        ctx_update_service::normalize_release_channel(q.channel.as_deref().unwrap_or("stable"))
            .map_err(|err| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: err.to_string(),
                    }),
                )
            })?;
    let base_url = ctx_update_service::default_download_base_url();
    let platform = ctx_update_service::platform_key().map(|s| s.to_string());
    let current_version = crate::build_identity::current_build_identity()
        .map(|identity| identity.exact_version.clone())
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            )
        })?;

    let query = platform.as_ref().map(|p| {
        vec![
            ("current_version", current_version.clone()),
            ("platform", p.clone()),
        ]
    });

    let manifest = ctx_update_service::fetch_latest_manifest_with_params(
        &base_url,
        &channel,
        query.as_deref(),
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let latest_version = manifest.latest_version.clone();
    let min_supported_version = manifest.min_supported_version.clone();
    let platform_supported = ctx_update_service::platform_supported(&manifest, platform.as_deref());
    let (in_place_update_supported, in_place_update_reason) =
        ctx_update_service::in_place_update_capability(
            &manifest,
            platform.as_deref(),
            platform_supported,
        );
    let update_available = ctx_update_service::is_update_available(
        &current_version,
        &latest_version,
        platform_supported,
    );

    Ok(Json(UpdateCheckResp {
        channel,
        base_url,
        platform,
        current_version,
        latest_version: Some(latest_version),
        min_supported_version,
        platform_supported,
        in_place_update_supported,
        in_place_update_reason,
        update_available,
        manifest: serde_json::to_value(manifest).unwrap_or(serde_json::Value::Null),
    }))
}
