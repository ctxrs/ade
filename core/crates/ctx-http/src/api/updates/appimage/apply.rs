use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use ctx_observability::logs;

use super::types::{ApplyAppImageReq, ApplyAppImageResp};
use crate::api::errors::ApiErrorResp;
use crate::daemon::CoreHandle;

pub(in crate::api) async fn apply_appimage_update(
    State(core): State<CoreHandle>,
    Json(req): Json<ApplyAppImageReq>,
) -> Result<Json<ApplyAppImageResp>, (StatusCode, Json<ApiErrorResp>)> {
    if !req.confirm {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "confirm required".to_string(),
            }),
        ));
    }

    let channel =
        ctx_update_service::normalize_release_channel(req.channel.as_deref().unwrap_or("stable"))
            .map_err(|err| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: err.to_string(),
                }),
            )
        })?;
    let base_url = ctx_update_service::default_download_base_url();
    let platform = ctx_update_service::platform_key().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "unsupported platform".to_string(),
            }),
        )
    })?;
    let Some(target) = ctx_update_service::appimage_path_env() else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "CTX_APPIMAGE_PATH not set; cannot apply in place".to_string(),
            }),
        ));
    };
    let current_version = ctx_update_service::current_build_identity(env!("CARGO_PKG_VERSION"))
        .map(|identity| identity.exact_version.clone())
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            )
        })?;
    let (downloaded, _meta) = ctx_update_service::validate_verified_appimage_candidate(
        core.data_root(),
        &target,
        &channel,
        platform,
        &base_url,
        &current_version,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    ctx_update_service::atomic_replace_file(&target, &downloaded)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    ctx_update_service::clear_appimage_candidate(core.data_root()).await;

    Ok(Json(ApplyAppImageResp {
        applied: true,
        target_path: Some(target.to_string_lossy().to_string()),
        message:
            "Update applied in place. Quit and relaunch the desktop app to run the new version."
                .to_string(),
    }))
}
