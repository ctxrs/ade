use super::*;

#[derive(Debug, Serialize)]
pub(super) struct UpdateCheckResp {
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
pub(super) struct UpdateCheckQuery {
    #[serde(default)]
    channel: Option<String>,
}

pub(super) async fn check_updates(
    State(_state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<UpdateCheckQuery>,
) -> Result<Json<UpdateCheckResp>, (StatusCode, Json<ApiErrorResp>)> {
    let channel = q.channel.unwrap_or_else(|| "stable".to_string());
    let base_url = crate::updates::default_download_base_url();
    let platform = crate::updates::platform_key().map(|s| s.to_string());
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

    let manifest =
        crate::updates::fetch_latest_manifest_with_params(&base_url, &channel, query.as_deref())
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
    let platform_supported = crate::updates::platform_supported(&manifest, platform.as_deref());
    let (in_place_update_supported, in_place_update_reason) =
        crate::updates::in_place_update_capability(
            &manifest,
            platform.as_deref(),
            platform_supported,
        );
    let update_available =
        crate::updates::is_update_available(&current_version, &latest_version, platform_supported);

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

#[derive(Debug, Serialize)]
pub(super) struct UpdateActivityResp {
    activity: crate::daemon::DaemonTurnActivitySummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    managed_daemon_auto_update: Option<crate::updates::ManagedDaemonAutoUpdateStatus>,
}

pub(super) async fn update_activity(
    State(state): State<Arc<AppState>>,
) -> Result<Json<UpdateActivityResp>, (StatusCode, Json<ApiErrorResp>)> {
    let activity = crate::daemon::daemon_turn_activity_summary(&state)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            )
        })?;
    let managed_daemon_auto_update =
        crate::updates::managed_daemon_auto_update_status_snapshot(&state.core.data_root).await;
    Ok(Json(UpdateActivityResp {
        activity,
        managed_daemon_auto_update,
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct BeginUpdateDrainReq {
    confirm: bool,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    owner: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct BeginUpdateDrainResp {
    acquired: bool,
    activity: crate::daemon::DaemonTurnActivitySummary,
}

pub(super) async fn begin_update_drain(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BeginUpdateDrainReq>,
) -> Result<Json<BeginUpdateDrainResp>, (StatusCode, Json<ApiErrorResp>)> {
    if !req.confirm {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "confirm required".to_string(),
            }),
        ));
    }
    let reason = req
        .reason
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "daemon_update".to_string());
    let owner = req
        .owner
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    if state.acquire_update_drain(reason, owner).await.is_none() {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "daemon update drain already active".to_string(),
            }),
        ));
    }
    let activity = crate::daemon::daemon_turn_activity_summary(&state)
        .await
        .map_err(|err| {
            let state = state.clone();
            tokio::spawn(async move {
                let _ = state.release_update_drain().await;
            });
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            )
        })?;
    if !activity.idle {
        let _ = state.release_update_drain().await;
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "daemon has queued or running turns; update drain was not acquired"
                    .to_string(),
            }),
        ));
    }
    Ok(Json(BeginUpdateDrainResp {
        acquired: true,
        activity,
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct ReleaseUpdateDrainReq {
    confirm: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct ReleaseUpdateDrainResp {
    released: bool,
}

pub(super) async fn release_update_drain(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ReleaseUpdateDrainReq>,
) -> Result<Json<ReleaseUpdateDrainResp>, (StatusCode, Json<ApiErrorResp>)> {
    if !req.confirm {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "confirm required".to_string(),
            }),
        ));
    }
    let released = state.release_update_drain().await;
    Ok(Json(ReleaseUpdateDrainResp { released }))
}

#[derive(Debug, Deserialize)]
pub(super) struct DownloadAppImageReq {
    #[serde(default)]
    channel: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DownloadAppImageResp {
    downloaded_path: String,
    can_apply_in_place: bool,
}

pub(super) async fn download_appimage_update(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DownloadAppImageReq>,
) -> Result<Json<DownloadAppImageResp>, (StatusCode, Json<ApiErrorResp>)> {
    let channel = req.channel.unwrap_or_else(|| "stable".to_string());
    let base_url = crate::updates::default_download_base_url();
    let platform = crate::updates::platform_key().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "unsupported platform".to_string(),
            }),
        )
    })?;

    let manifest = crate::updates::fetch_latest_manifest(&base_url, &channel)
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

    let target_path = crate::updates::appimage_path_env().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "CTX_APPIMAGE_PATH not set; cannot apply in place".to_string(),
            }),
        )
    })?;
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
    let url = crate::updates::join_url(&base_url, &appimage.url_path);
    let manifest_url = crate::updates::release_manifest_url(&base_url, &channel);
    let meta = crate::updates::download_verified_appimage_candidate(
        crate::updates::AppImageCandidateRequest {
            data_root: &state.core.data_root,
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
        can_apply_in_place: crate::updates::appimage_path_env().is_some(),
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct ApplyAppImageReq {
    confirm: bool,
    #[serde(default)]
    channel: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ApplyAppImageResp {
    applied: bool,
    target_path: Option<String>,
    message: String,
}

pub(super) async fn apply_appimage_update(
    State(state): State<Arc<AppState>>,
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

    let channel = req.channel.unwrap_or_else(|| "stable".to_string());
    let base_url = crate::updates::default_download_base_url();
    let platform = crate::updates::platform_key().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "unsupported platform".to_string(),
            }),
        )
    })?;
    let Some(target) = crate::updates::appimage_path_env() else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "CTX_APPIMAGE_PATH not set; cannot apply in place".to_string(),
            }),
        ));
    };
    let (downloaded, _meta) = crate::updates::validate_verified_appimage_candidate(
        &state.core.data_root,
        &target,
        &channel,
        platform,
        &base_url,
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

    crate::updates::atomic_replace_file(&target, &downloaded)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    crate::updates::clear_appimage_candidate(&state.core.data_root).await;

    Ok(Json(ApplyAppImageResp {
        applied: true,
        target_path: Some(target.to_string_lossy().to_string()),
        message:
            "Update applied in place. Quit and relaunch the desktop app to run the new version."
                .to_string(),
    }))
}
