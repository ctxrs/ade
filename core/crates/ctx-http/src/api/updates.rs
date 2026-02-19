use super::*;

#[derive(Debug, Serialize)]
pub(super) struct UpdateCheckResp {
    channel: String,
    base_url: String,
    platform: Option<String>,
    current_version: String,
    latest_version: Option<String>,
    platform_supported: bool,
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
    let current_version = env!("CARGO_PKG_VERSION").to_string();

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
    let platform_supported = platform_supported(&manifest, platform.as_deref());
    let update_available =
        is_update_available(&current_version, &latest_version, platform_supported);

    Ok(Json(UpdateCheckResp {
        channel,
        base_url,
        platform,
        current_version,
        latest_version: Some(latest_version),
        platform_supported,
        update_available,
        manifest: serde_json::to_value(manifest).unwrap_or(serde_json::Value::Null),
    }))
}

fn platform_supported(manifest: &crate::updates::ReleaseManifest, platform_key: Option<&str>) -> bool {
    let Some(platform_key) = platform_key else {
        return false;
    };
    let Some(entry) = manifest.platforms.get(platform_key) else {
        return false;
    };
    entry.preferred_desktop_artifact(platform_key).is_some()
}

fn is_update_available(current_version: &str, latest_version: &str, platform_supported: bool) -> bool {
    if !platform_supported {
        return false;
    }
    match (
        crate::updates::normalize_version_str(current_version),
        crate::updates::normalize_version_str(latest_version),
    ) {
        (Some(cur), Some(lat)) => lat > cur,
        _ => false,
    }
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

    let url = crate::updates::join_url(&base_url, &appimage.url_path);
    let dest = crate::updates::updates_dir(&state.core.data_root).join("ctx.AppImage.new");
    crate::updates::download_and_verify(&url, &appimage.sha256, &dest)
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
        if let Ok(md) = tokio::fs::metadata(&dest).await {
            let mut p = md.permissions();
            p.set_mode(0o755);
            let _ = tokio::fs::set_permissions(&dest, p).await;
        }
    }

    Ok(Json(DownloadAppImageResp {
        downloaded_path: dest.to_string_lossy().to_string(),
        can_apply_in_place: crate::updates::appimage_path_env().is_some(),
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct ApplyAppImageReq {
    confirm: bool,
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

    let Some(target) = crate::updates::appimage_path_env() else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "CTX_APPIMAGE_PATH not set; cannot apply in place".to_string(),
            }),
        ));
    };
    let downloaded = crate::updates::updates_dir(&state.core.data_root).join("ctx.AppImage.new");
    if !downloaded.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "no downloaded update found; call download first".to_string(),
            }),
        ));
    }

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

    Ok(Json(ApplyAppImageResp {
        applied: true,
        target_path: Some(target.to_string_lossy().to_string()),
        message:
            "Update applied in place. Quit and relaunch the desktop app to run the new version."
                .to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn artifact(name: &str) -> crate::updates::ReleaseArtifact {
        crate::updates::ReleaseArtifact {
            url_path: format!("/{name}"),
            sha256: "abc123".to_string(),
        }
    }

    fn release_platform_with_dmg() -> crate::updates::ReleasePlatform {
        crate::updates::ReleasePlatform {
            desktop: None,
            appimage: None,
            deb: None,
            dmg: Some(artifact("ctx.dmg")),
            msi: None,
            nsis: None,
            exe: None,
            zip: None,
            daemon: Some(artifact("ctx-daemon")),
        }
    }

    fn release_platform_daemon_only() -> crate::updates::ReleasePlatform {
        crate::updates::ReleasePlatform {
            desktop: None,
            appimage: None,
            deb: None,
            dmg: None,
            msi: None,
            nsis: None,
            exe: None,
            zip: None,
            daemon: Some(artifact("ctx-daemon")),
        }
    }

    fn manifest_with_platforms(
        platforms: HashMap<String, crate::updates::ReleasePlatform>,
    ) -> crate::updates::ReleaseManifest {
        crate::updates::ReleaseManifest {
            channel: "stable".to_string(),
            latest_version: "1.2.3".to_string(),
            published_at: "2026-02-19T00:00:00Z".to_string(),
            platforms,
        }
    }

    #[test]
    fn platform_supported_false_when_platform_missing() {
        let manifest = manifest_with_platforms(HashMap::new());
        assert!(!platform_supported(&manifest, Some("macos-arm64")));
    }

    #[test]
    fn platform_supported_false_when_no_desktop_artifact() {
        let mut platforms = HashMap::new();
        platforms.insert("macos-arm64".to_string(), release_platform_daemon_only());
        let manifest = manifest_with_platforms(platforms);
        assert!(!platform_supported(&manifest, Some("macos-arm64")));
    }

    #[test]
    fn platform_supported_true_with_matching_desktop_artifact() {
        let mut platforms = HashMap::new();
        platforms.insert("macos-arm64".to_string(), release_platform_with_dmg());
        let manifest = manifest_with_platforms(platforms);
        assert!(platform_supported(&manifest, Some("macos-arm64")));
    }

    #[test]
    fn update_available_requires_supported_platform() {
        assert!(!is_update_available("1.0.0", "1.2.3", false));
        assert!(is_update_available("1.0.0", "1.2.3", true));
    }
}
