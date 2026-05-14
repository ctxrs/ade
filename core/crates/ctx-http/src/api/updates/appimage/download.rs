use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use ctx_observability::logs;

use super::types::{DownloadAppImageReq, DownloadAppImageResp};
use crate::api::errors::ApiErrorResp;
use ctx_daemon::daemon::CoreHandle;

pub(in crate::api) async fn download_appimage_update(
    State(core): State<CoreHandle>,
    Json(req): Json<DownloadAppImageReq>,
) -> Result<Json<DownloadAppImageResp>, (StatusCode, Json<ApiErrorResp>)> {
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

    let manifest = ctx_update_service::fetch_latest_manifest(&base_url, &channel)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let platform_entry = manifest.platforms.get(platform).ok_or_else(|| {
        (
            StatusCode::BAD_GATEWAY,
            Json(ApiErrorResp {
                error: format!("manifest missing platform {platform}"),
            }),
        )
    })?;
    let appimage = platform_entry.appimage.as_ref().ok_or_else(|| {
        (
            StatusCode::BAD_GATEWAY,
            Json(ApiErrorResp {
                error: "manifest missing appimage artifact".to_string(),
            }),
        )
    })?;

    let target_path = ctx_update_service::appimage_path_env().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "CTX_APPIMAGE_PATH not set; cannot apply in place".to_string(),
            }),
        )
    })?;
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
    let url = ctx_update_service::resolve_release_artifact_url(&base_url, &appimage.url_path)
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let manifest_url = ctx_update_service::release_manifest_url(&base_url, &channel);
    let meta = ctx_update_service::download_verified_appimage_candidate(
        ctx_update_service::AppImageCandidateRequest {
            data_root: core.data_root(),
            target_path: &target_path,
            channel: &channel,
            platform,
            target_version: &manifest.latest_version,
            current_version: &current_version,
            artifact_url: &url,
            artifact_url_path: &appimage.url_path,
            manifest_url: &manifest_url,
            base_url: &base_url,
            sha256: &appimage.sha256,
        },
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

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(md) = tokio::fs::metadata(&meta.candidate_path).await {
            let mut p = md.permissions();
            p.set_mode(0o755);
            let _ = tokio::fs::set_permissions(&meta.candidate_path, p).await;
        }
    }

    Ok(Json(DownloadAppImageResp {
        downloaded_path: meta.candidate_path.to_string_lossy().to_string(),
        can_apply_in_place: ctx_update_service::appimage_path_env().is_some(),
    }))
}
