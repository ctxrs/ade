use super::*;

use std::path::Path;
use tauri_plugin_updater::UpdaterExt;

pub(super) fn clear_staged_update_files(meta_path: &Path, bytes_path: &Path) -> Result<(), String> {
    if meta_path.exists() {
        std::fs::remove_file(meta_path).map_err(|e| {
            format!(
                "clearing staged update metadata '{}': {e}",
                meta_path.display()
            )
        })?;
    }
    if bytes_path.exists() {
        std::fs::remove_file(bytes_path).map_err(|e| {
            format!(
                "clearing staged update bytes '{}': {e}",
                bytes_path.display()
            )
        })?;
    }
    Ok(())
}

fn staged_update_meta_is_valid(meta: &DesktopStagedUpdateMeta) -> bool {
    !meta.version.trim().is_empty()
        && !meta.target.trim().is_empty()
        && !meta.endpoint.trim().is_empty()
        && !meta.channel.trim().is_empty()
}

pub(super) fn read_staged_update_meta(
    meta_path: &Path,
    bytes_path: &Path,
) -> Result<Option<DesktopStagedUpdateMeta>, String> {
    if !meta_path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(meta_path).map_err(|e| {
        format!(
            "reading staged update metadata '{}': {e}",
            meta_path.display()
        )
    })?;
    let parsed = match serde_json::from_str::<DesktopStagedUpdateMeta>(&raw) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!(
                "warn: clearing corrupt staged update metadata '{}': {err}",
                meta_path.display()
            );
            clear_staged_update_files(meta_path, bytes_path).map_err(|clear_err| {
                format!(
                    "parsing staged update metadata '{}': {err}; clearing corrupt staged update state: {clear_err}",
                    meta_path.display()
                )
            })?;
            return Ok(None);
        }
    };
    if !staged_update_meta_is_valid(&parsed) {
        clear_staged_update_files(meta_path, bytes_path)?;
        return Ok(None);
    }
    Ok(Some(parsed))
}

pub(super) fn write_staged_update_files(
    meta_path: &Path,
    bytes_path: &Path,
    meta: &DesktopStagedUpdateMeta,
    bytes: &[u8],
) -> Result<(), String> {
    std::fs::write(bytes_path, bytes).map_err(|e| {
        format!(
            "writing staged update bytes '{}': {e}",
            bytes_path.display()
        )
    })?;
    let encoded = serde_json::to_string_pretty(meta)
        .map_err(|e| format!("encoding staged update metadata: {e}"))?;
    std::fs::write(meta_path, format!("{encoded}\n")).map_err(|e| {
        format!(
            "writing staged update metadata '{}': {e}",
            meta_path.display()
        )
    })
}

pub(super) fn read_staged_update_meta_for_app(
    app: &tauri::AppHandle,
) -> Result<Option<DesktopStagedUpdateMeta>, String> {
    let path = staged_meta_path_for_app(app)?;
    let bytes_path = staged_bytes_path_for_app(app)?;
    read_staged_update_meta(&path, &bytes_path)
}

fn write_staged_update_for_app(
    app: &tauri::AppHandle,
    meta: &DesktopStagedUpdateMeta,
    bytes: &[u8],
) -> Result<(), String> {
    let bytes_path = staged_bytes_path_for_app(app)?;
    let meta_path = staged_meta_path_for_app(app)?;
    write_staged_update_files(&meta_path, &bytes_path, meta, bytes)
}

pub(super) fn clear_staged_update_for_app(app: &tauri::AppHandle) -> Result<(), String> {
    let meta_path = staged_meta_path_for_app(app)?;
    let bytes_path = staged_bytes_path_for_app(app)?;
    clear_staged_update_files(&meta_path, &bytes_path)
}

pub(super) fn clear_staged_update_if_current_version_is_new_enough(
    app: &tauri::AppHandle,
    current_version: &str,
) -> Result<(), String> {
    let Some(meta) = read_staged_update_meta_for_app(app)? else {
        return Ok(());
    };
    if support::version_is_at_or_above(current_version, &meta.version) {
        clear_staged_update_for_app(app)?;
    }
    Ok(())
}

pub(super) fn has_matching_staged_update(
    app: &tauri::AppHandle,
    channel: &str,
    expected_version: &str,
    config: &DesktopNativeUpdaterConfig,
) -> Result<bool, String> {
    let meta_path = staged_meta_path_for_app(app)?;
    let bytes_path = staged_bytes_path_for_app(app)?;
    has_matching_staged_update_paths(&meta_path, &bytes_path, channel, expected_version, config)
}

pub(super) fn has_matching_staged_update_paths(
    meta_path: &Path,
    bytes_path: &Path,
    channel: &str,
    expected_version: &str,
    config: &DesktopNativeUpdaterConfig,
) -> Result<bool, String> {
    let Some(meta) = read_staged_update_meta(meta_path, bytes_path)? else {
        return Ok(false);
    };
    let matches_expected = meta.version.trim() == expected_version.trim()
        && meta.target.trim() == config.target.trim()
        && meta.endpoint.trim() == config.endpoint.trim()
        && meta.channel.trim() == channel.trim();
    if !matches_expected {
        clear_staged_update_files(meta_path, bytes_path)?;
        return Ok(false);
    }
    let exists = bytes_path.exists();
    if !exists {
        clear_staged_update_files(meta_path, bytes_path)?;
    }
    Ok(exists)
}

pub(super) fn read_staged_update_bytes_if_matching(
    app: &tauri::AppHandle,
    channel: &str,
    expected_version: &str,
    config: &DesktopNativeUpdaterConfig,
) -> Result<Option<Vec<u8>>, String> {
    let meta_path = staged_meta_path_for_app(app)?;
    let bytes_path = staged_bytes_path_for_app(app)?;
    read_staged_update_bytes_if_matching_paths(
        &meta_path,
        &bytes_path,
        channel,
        expected_version,
        config,
    )
}

pub(super) fn read_staged_update_bytes_if_matching_paths(
    meta_path: &Path,
    bytes_path: &Path,
    channel: &str,
    expected_version: &str,
    config: &DesktopNativeUpdaterConfig,
) -> Result<Option<Vec<u8>>, String> {
    if !has_matching_staged_update_paths(meta_path, bytes_path, channel, expected_version, config)?
    {
        return Ok(None);
    }
    let bytes = std::fs::read(bytes_path).map_err(|e| {
        format!(
            "reading staged update bytes '{}': {e}",
            bytes_path.display()
        )
    })?;
    if bytes.is_empty() {
        clear_staged_update_files(meta_path, bytes_path)?;
        return Ok(None);
    }
    Ok(Some(bytes))
}

pub(super) async fn stage_update_in_background(
    app: tauri::AppHandle,
    channel: &str,
) -> Result<(), String> {
    let current_version = app.package_info().version.to_string();
    let mut attempt = attempts::begin_update_attempt(channel, &current_version);

    let config = support::resolve_native_updater_config(channel)?;
    let Some(pubkey) = config.pubkey.as_deref() else {
        clear_staged_update_for_app(&app)?;
        attempts::persist_attempt_success_best_effort(&app, &mut attempt);
        return Ok(());
    };
    let endpoint_url = support::endpoint_with_download_id(&config.endpoint, None)?;
    let build_stage = attempts::begin_attempt_stage(&mut attempt, "build");
    let updater = app
        .updater_builder()
        .target(config.target.clone())
        .pubkey(pubkey)
        .endpoints(vec![endpoint_url])
        .map_err(|e| {
            let err = support::updater_stage_error("build", e);
            attempts::fail_attempt_stage(&mut attempt, build_stage, "build", &err);
            attempts::persist_attempt_failure_best_effort(&app, &mut attempt, err)
        })?
        .build()
        .map_err(|e| {
            let err = support::updater_stage_error("build", e);
            attempts::fail_attempt_stage(&mut attempt, build_stage, "build", &err);
            attempts::persist_attempt_failure_best_effort(&app, &mut attempt, err)
        })?;
    attempts::complete_attempt_stage(&mut attempt, build_stage);

    let check_stage = attempts::begin_attempt_stage(&mut attempt, "check");
    let Some(update) = updater.check().await.map_err(|e| {
        let err = support::updater_stage_error("check", e);
        attempts::fail_attempt_stage(&mut attempt, check_stage, "check", &err);
        attempts::persist_attempt_failure_best_effort(&app, &mut attempt, err)
    })?
    else {
        attempts::complete_attempt_stage(&mut attempt, check_stage);
        clear_staged_update_for_app(&app)?;
        attempts::persist_attempt_success_best_effort(&app, &mut attempt);
        return Ok(());
    };
    attempts::complete_attempt_stage(&mut attempt, check_stage);
    let latest_version = update.version.clone();
    attempt.target_version = Some(latest_version.clone());

    let verify_stage = attempts::begin_attempt_stage(&mut attempt, "verify");
    if !support::version_is_strictly_newer(&latest_version, &current_version) {
        attempts::complete_attempt_stage(&mut attempt, verify_stage);
        clear_staged_update_for_app(&app)?;
        attempts::persist_attempt_success_best_effort(&app, &mut attempt);
        return Ok(());
    }
    attempts::complete_attempt_stage(&mut attempt, verify_stage);

    let download_stage = attempts::begin_attempt_stage(&mut attempt, "download");
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| {
        let err = support::updater_stage_error("download", e);
        attempts::fail_attempt_stage(&mut attempt, download_stage, "download", &err);
        attempts::persist_attempt_failure_best_effort(&app, &mut attempt, err)
    })?;
    attempts::complete_attempt_stage(&mut attempt, download_stage);

    let marker_stage = attempts::begin_attempt_stage(&mut attempt, "marker");
    let meta = DesktopStagedUpdateMeta {
        version: latest_version,
        target: config.target,
        endpoint: config.endpoint,
        channel: channel.to_string(),
        downloaded_at_ms: now_ms(),
        size_bytes: bytes.len(),
    };
    write_staged_update_for_app(&app, &meta, &bytes).map_err(|e| {
        let err = support::updater_stage_error("marker_write", e);
        attempts::fail_attempt_stage(&mut attempt, marker_stage, "marker_write", &err);
        attempts::persist_attempt_failure_best_effort(&app, &mut attempt, err)
    })?;
    attempts::complete_attempt_stage(&mut attempt, marker_stage);
    attempts::persist_attempt_success_best_effort(&app, &mut attempt);
    Ok(())
}
