use super::*;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri_plugin_updater::UpdaterExt;
use url::Url;

const RESTART_MARKER_FILENAME: &str = "desktop_update_restart_required.json";
const STAGED_UPDATE_META_FILENAME: &str = "desktop_update_staged.v1.json";
const STAGED_UPDATE_BYTES_FILENAME: &str = "desktop_update_staged.v1.bin";
const LAST_ATTEMPT_FILENAME: &str = "desktop_update_attempt_last.v1.json";
const RESTART_READY_MESSAGE: &str =
    "Update takes ~1 second and preserves data. Active agents will be paused.";
static STAGING_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DesktopAppUpdatePhase {
    Idle,
    Staging,
    StagedReady,
    RestartRequired,
    Failed,
}

impl DesktopAppUpdatePhase {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Staging => "staging",
            Self::StagedReady => "staged_ready",
            Self::RestartRequired => "restart_required",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DesktopStagedUpdateMeta {
    version: String,
    target: String,
    endpoint: String,
    channel: String,
    downloaded_at_ms: u64,
    size_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DesktopUpdateAttemptResult {
    InProgress,
    Succeeded,
    Failed,
}

impl DesktopUpdateAttemptResult {
    fn as_str(&self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DesktopUpdateAttemptStage {
    stage: String,
    started_at_ms: u64,
    finished_at_ms: Option<u64>,
    result: DesktopUpdateAttemptResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DesktopUpdateAttempt {
    attempt_id: String,
    channel: String,
    current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_version: Option<String>,
    started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at_ms: Option<u64>,
    result: DesktopUpdateAttemptResult,
    stages: Vec<DesktopUpdateAttemptStage>,
}

#[derive(Debug, Serialize)]
pub(super) struct DesktopAppUpdateAttemptResp {
    attempt_id: String,
    channel: String,
    current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_version: Option<String>,
    started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at_ms: Option<u64>,
    result: String,
    stages: Vec<DesktopAppUpdateAttemptStageResp>,
}

#[derive(Debug, Serialize)]
pub(super) struct DesktopAppUpdateAttemptStageResp {
    stage: String,
    started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at_ms: Option<u64>,
    result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DesktopAppUpdateCheckReq {
    #[serde(default)]
    channel: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DesktopAppUpdateCheckResp {
    configured: bool,
    available: bool,
    restart_required: bool,
    phase: String,
    staged: bool,
    current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_version: Option<String>,
    target: String,
    endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_attempt_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DesktopAppUpdateStateResp {
    configured: bool,
    available: bool,
    restart_required: bool,
    phase: String,
    staged: bool,
    current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_version: Option<String>,
    target: String,
    endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_attempt_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DesktopAppUpdateApplyReq {
    #[serde(default)]
    confirm: bool,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    download_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DesktopAppUpdateApplyResp {
    applied: bool,
    needs_restart: bool,
    up_to_date: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_version: Option<String>,
    message: String,
}

#[derive(Debug, Serialize)]
pub(super) struct DesktopAppRestartResp {
    requested: bool,
    message: String,
}

#[derive(Debug, Clone)]
struct DesktopNativeUpdaterConfig {
    target: String,
    endpoint: String,
    pubkey: Option<String>,
}

impl From<DesktopUpdateAttempt> for DesktopAppUpdateAttemptResp {
    fn from(value: DesktopUpdateAttempt) -> Self {
        Self {
            attempt_id: value.attempt_id,
            channel: value.channel,
            current_version: value.current_version,
            target_version: value.target_version,
            started_at_ms: value.started_at_ms,
            finished_at_ms: value.finished_at_ms,
            result: value.result.as_str().to_string(),
            stages: value
                .stages
                .into_iter()
                .map(DesktopAppUpdateAttemptStageResp::from)
                .collect(),
        }
    }
}

impl From<DesktopUpdateAttemptStage> for DesktopAppUpdateAttemptStageResp {
    fn from(value: DesktopUpdateAttemptStage) -> Self {
        Self {
            stage: value.stage,
            started_at_ms: value.started_at_ms,
            finished_at_ms: value.finished_at_ms,
            result: value.result.as_str().to_string(),
            error_code: value.error_code,
            error_message: value.error_message,
        }
    }
}

#[tauri::command]
pub(super) async fn desktop_check_app_update(
    app: tauri::AppHandle,
    req: DesktopAppUpdateCheckReq,
) -> Result<DesktopAppUpdateCheckResp, String> {
    let state = desktop_get_app_update_state(app, req).await?;
    Ok(DesktopAppUpdateCheckResp {
        configured: state.configured,
        available: state.available,
        restart_required: state.restart_required,
        phase: state.phase,
        staged: state.staged,
        current_version: state.current_version,
        latest_version: state.latest_version,
        target: state.target,
        endpoint: state.endpoint,
        message: state.message,
        last_attempt_id: state.last_attempt_id,
        last_error: state.last_error,
    })
}

#[tauri::command]
pub(super) async fn desktop_get_app_update_state(
    app: tauri::AppHandle,
    req: DesktopAppUpdateCheckReq,
) -> Result<DesktopAppUpdateStateResp, String> {
    let channel = desktop_ssh::normalize_update_channel(req.channel.as_deref())?;
    resolve_desktop_update_state(&app, &channel).await
}

#[tauri::command]
pub(super) fn desktop_get_last_app_update_attempt(
    app: tauri::AppHandle,
) -> Result<Option<DesktopAppUpdateAttemptResp>, String> {
    let raw = read_last_attempt_for_app(&app)?;
    Ok(raw.map(DesktopAppUpdateAttemptResp::from))
}

#[tauri::command]
pub(super) async fn desktop_apply_app_update(
    app: tauri::AppHandle,
    req: DesktopAppUpdateApplyReq,
) -> Result<DesktopAppUpdateApplyResp, String> {
    if !req.confirm {
        return Err("confirm required".to_string());
    }
    let channel = desktop_ssh::normalize_update_channel(req.channel.as_deref())?;
    let download_id = normalize_download_id(req.download_id.as_deref());
    let pre_state = resolve_desktop_update_state(&app, &channel).await?;

    if pre_state.restart_required {
        return Ok(DesktopAppUpdateApplyResp {
            applied: false,
            needs_restart: true,
            up_to_date: false,
            latest_version: pre_state.latest_version,
            message: RESTART_READY_MESSAGE.to_string(),
        });
    }

    if !pre_state.configured {
        return Err(
            "native updater is not configured (missing CTX_DESKTOP_UPDATER_PUBKEY)".to_string(),
        );
    }

    let mut attempt = begin_update_attempt(&channel, &pre_state.current_version);

    let config = resolve_native_updater_config(&channel)?;
    let pubkey = config.pubkey.as_deref().ok_or_else(|| {
        "native updater is not configured (missing CTX_DESKTOP_UPDATER_PUBKEY)".to_string()
    })?;
    let endpoint_url = endpoint_with_download_id(&config.endpoint, download_id.as_deref())?;
    let build_stage = begin_attempt_stage(&mut attempt, "build");
    let updater = app
        .updater_builder()
        .target(config.target.clone())
        .pubkey(pubkey)
        .endpoints(vec![endpoint_url])
        .map_err(|e| {
            let err = updater_stage_error("build", e);
            fail_attempt_stage(&mut attempt, build_stage, "build", &err);
            persist_attempt_failure_best_effort(&app, &mut attempt, err)
        })?
        .build()
        .map_err(|e| {
            let err = updater_stage_error("build", e);
            fail_attempt_stage(&mut attempt, build_stage, "build", &err);
            persist_attempt_failure_best_effort(&app, &mut attempt, err)
        })?;
    complete_attempt_stage(&mut attempt, build_stage);

    let check_stage = begin_attempt_stage(&mut attempt, "check");
    let Some(update) = updater.check().await.map_err(|e| {
        let err = updater_stage_error("check", e);
        fail_attempt_stage(&mut attempt, check_stage, "check", &err);
        persist_attempt_failure_best_effort(&app, &mut attempt, err)
    })?
    else {
        complete_attempt_stage(&mut attempt, check_stage);
        persist_attempt_success_best_effort(&app, &mut attempt);
        return Ok(DesktopAppUpdateApplyResp {
            applied: false,
            needs_restart: false,
            up_to_date: true,
            latest_version: None,
            message: "No desktop app update is currently available.".to_string(),
        });
    };
    complete_attempt_stage(&mut attempt, check_stage);
    let latest_version = update.version.clone();
    attempt.target_version = Some(latest_version.clone());
    eprintln!(
        "native updater apply start: target={} version={latest_version}",
        config.target
    );

    let verify_stage = begin_attempt_stage(&mut attempt, "verify");
    if !version_is_strictly_newer(&latest_version, &pre_state.current_version) {
        complete_attempt_stage(&mut attempt, verify_stage);
        persist_attempt_success_best_effort(&app, &mut attempt);
        return Ok(DesktopAppUpdateApplyResp {
            applied: false,
            needs_restart: false,
            up_to_date: true,
            latest_version: Some(latest_version),
            message: "No desktop app update is currently available.".to_string(),
        });
    }
    complete_attempt_stage(&mut attempt, verify_stage);

    let download_stage = begin_attempt_stage(&mut attempt, "download");
    let bytes = if let Some(staged_bytes) =
        read_staged_update_bytes_if_matching(&app, &channel, &latest_version, &config)?
    {
        complete_attempt_stage(&mut attempt, download_stage);
        staged_bytes
    } else {
        let fresh = update.download(|_, _| {}, || {}).await.map_err(|e| {
            let err = updater_stage_error("download", e);
            fail_attempt_stage(&mut attempt, download_stage, "download", &err);
            persist_attempt_failure_best_effort(&app, &mut attempt, err)
        })?;
        complete_attempt_stage(&mut attempt, download_stage);
        fresh
    };

    let install_stage = begin_attempt_stage(&mut attempt, "install");
    update.install(&bytes).map_err(|e| {
        let err = updater_stage_error("install", e);
        fail_attempt_stage(&mut attempt, install_stage, "install", &err);
        persist_attempt_failure_best_effort(&app, &mut attempt, err)
    })?;
    complete_attempt_stage(&mut attempt, install_stage);

    let marker_stage = begin_attempt_stage(&mut attempt, "marker");
    write_restart_marker_for_app(&app, &latest_version).map_err(|e| {
        let err = updater_stage_error("marker_write", e);
        fail_attempt_stage(&mut attempt, marker_stage, "marker_write", &err);
        persist_attempt_failure_best_effort(&app, &mut attempt, err)
    })?;
    complete_attempt_stage(&mut attempt, marker_stage);
    if let Err(err) = clear_staged_update_for_app(&app) {
        eprintln!("warn: failed to clear staged updater payload after install: {err}");
    }
    eprintln!("native updater apply success: version={latest_version}");
    persist_attempt_success_best_effort(&app, &mut attempt);

    Ok(DesktopAppUpdateApplyResp {
        applied: true,
        needs_restart: true,
        up_to_date: false,
        latest_version: Some(latest_version),
        message: RESTART_READY_MESSAGE.to_string(),
    })
}

#[tauri::command]
pub(super) fn desktop_restart_app(app: tauri::AppHandle) -> Result<DesktopAppRestartResp, String> {
    if let Ok(Some(mut attempt)) = read_last_attempt_for_app(&app) {
        let stage = begin_attempt_stage(&mut attempt, "restart");
        complete_attempt_stage(&mut attempt, stage);
        if let Err(err) = write_last_attempt_for_app(&app, &attempt) {
            eprintln!("warn: failed to persist updater restart stage: {err}");
        }
    }
    let app_handle = app.clone();
    thread::spawn(move || {
        // Allow the invoke response to flush before requesting restart.
        thread::sleep(Duration::from_millis(80));
        app_handle.request_restart();
    });
    Ok(DesktopAppRestartResp {
        requested: true,
        message: "Restart requested.".to_string(),
    })
}

async fn resolve_desktop_update_state(
    app: &tauri::AppHandle,
    channel: &str,
) -> Result<DesktopAppUpdateStateResp, String> {
    let current_version = app.package_info().version.to_string();
    let config = resolve_native_updater_config(channel)?;
    clear_staged_update_if_current_version_is_new_enough(app, &current_version)?;
    let pending_restart_version = reconcile_restart_marker_for_app(app, &current_version)?;
    let restart_required = pending_restart_version.is_some();
    let last_attempt = read_last_attempt_for_app(app)?;
    let last_attempt_id = last_attempt.as_ref().map(|entry| entry.attempt_id.clone());
    let mut last_error = last_attempt
        .as_ref()
        .and_then(last_failed_stage_message)
        .map(|value| value.to_string());
    let message = if config.pubkey.is_none() {
        Some("Native updater is not configured (missing CTX_DESKTOP_UPDATER_PUBKEY).".to_string())
    } else {
        None
    };
    if restart_required {
        return Ok(DesktopAppUpdateStateResp {
            configured: config.pubkey.is_some(),
            available: true,
            restart_required: true,
            phase: DesktopAppUpdatePhase::RestartRequired.as_str().to_string(),
            staged: false,
            current_version,
            latest_version: pending_restart_version,
            target: config.target,
            endpoint: config.endpoint,
            message: Some(RESTART_READY_MESSAGE.to_string()),
            last_attempt_id,
            last_error,
        });
    }
    let Some(pubkey) = config.pubkey.as_deref() else {
        return Ok(DesktopAppUpdateStateResp {
            configured: false,
            available: false,
            restart_required,
            phase: DesktopAppUpdatePhase::Idle.as_str().to_string(),
            staged: false,
            current_version,
            latest_version: pending_restart_version,
            target: config.target,
            endpoint: config.endpoint,
            message,
            last_attempt_id,
            last_error,
        });
    };
    let endpoint_url =
        Url::parse(&config.endpoint).map_err(|e| format!("invalid update endpoint: {e}"))?;
    let updater = app
        .updater_builder()
        .target(config.target.clone())
        .pubkey(pubkey)
        .endpoints(vec![endpoint_url])
        .map_err(to_err)?
        .build()
        .map_err(to_err)?;
    let update = updater.check().await.map_err(|e| {
        let msg = updater_stage_error("check", e);
        msg
    })?;
    let raw_latest_version = update.as_ref().map(|v| v.version.clone());
    let latest_from_feed = normalize_latest_version(
        &current_version,
        raw_latest_version.as_deref(),
        pending_restart_version.as_deref(),
    );
    let latest = raw_latest_version
        .as_deref()
        .filter(|latest| version_is_strictly_newer(latest, &current_version))
        .map(|v| v.to_string());
    if latest.is_none() {
        clear_staged_update_for_app(app)?;
        return Ok(DesktopAppUpdateStateResp {
            configured: true,
            available: false,
            restart_required: false,
            phase: DesktopAppUpdatePhase::Idle.as_str().to_string(),
            staged: false,
            current_version,
            latest_version: latest_from_feed,
            target: config.target,
            endpoint: config.endpoint,
            message: None,
            last_attempt_id,
            last_error: None,
        });
    }
    let latest = latest.unwrap_or_default();
    let staged_ready = has_matching_staged_update(app, channel, &latest, &config)?;
    if staged_ready {
        return Ok(DesktopAppUpdateStateResp {
            configured: true,
            available: true,
            restart_required: false,
            phase: DesktopAppUpdatePhase::StagedReady.as_str().to_string(),
            staged: true,
            current_version,
            latest_version: Some(latest),
            target: config.target,
            endpoint: config.endpoint,
            message: None,
            last_attempt_id,
            last_error: None,
        });
    }
    if !STAGING_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        let app_handle = app.clone();
        let channel_owned = channel.to_string();
        tauri::async_runtime::spawn(async move {
            let result = stage_update_in_background(app_handle, &channel_owned).await;
            if let Err(err) = result {
                eprintln!("warn: background desktop updater staging failed: {err}");
            }
            STAGING_IN_PROGRESS.store(false, Ordering::SeqCst);
        });
    }
    if let Some(attempt) = &last_attempt {
        if attempt.result == DesktopUpdateAttemptResult::Failed
            && attempt.target_version.as_deref() == Some(latest.as_str())
        {
            last_error = last_failed_stage_message(attempt).map(|value| value.to_string());
        }
    }
    Ok(DesktopAppUpdateStateResp {
        configured: true,
        available: false,
        restart_required: false,
        phase: DesktopAppUpdatePhase::Staging.as_str().to_string(),
        staged: false,
        current_version,
        latest_version: Some(latest),
        target: config.target,
        endpoint: config.endpoint,
        message: Some("Downloading update in background.".to_string()),
        last_attempt_id,
        last_error,
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn app_data_root_for_app(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolving app_data_dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating app_data_dir: {e}"))?;
    Ok(dir)
}

fn staged_meta_path_for_app(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_root_for_app(app)?.join(STAGED_UPDATE_META_FILENAME))
}

fn staged_bytes_path_for_app(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_root_for_app(app)?.join(STAGED_UPDATE_BYTES_FILENAME))
}

fn last_attempt_path_for_app(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_root_for_app(app)?.join(LAST_ATTEMPT_FILENAME))
}

fn read_staged_update_meta_for_app(
    app: &tauri::AppHandle,
) -> Result<Option<DesktopStagedUpdateMeta>, String> {
    let path = staged_meta_path_for_app(app)?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("reading staged update metadata '{}': {e}", path.display()))?;
    let parsed = serde_json::from_str::<DesktopStagedUpdateMeta>(&raw)
        .map_err(|e| format!("parsing staged update metadata '{}': {e}", path.display()))?;
    if parsed.version.trim().is_empty() {
        clear_staged_update_for_app(app)?;
        return Ok(None);
    }
    Ok(Some(parsed))
}

fn write_staged_update_for_app(
    app: &tauri::AppHandle,
    meta: &DesktopStagedUpdateMeta,
    bytes: &[u8],
) -> Result<(), String> {
    let bytes_path = staged_bytes_path_for_app(app)?;
    let meta_path = staged_meta_path_for_app(app)?;
    std::fs::write(&bytes_path, bytes).map_err(|e| {
        format!(
            "writing staged update bytes '{}': {e}",
            bytes_path.display()
        )
    })?;
    let encoded = serde_json::to_string_pretty(meta)
        .map_err(|e| format!("encoding staged update metadata: {e}"))?;
    std::fs::write(&meta_path, format!("{encoded}\n")).map_err(|e| {
        format!(
            "writing staged update metadata '{}': {e}",
            meta_path.display()
        )
    })
}

fn clear_staged_update_for_app(app: &tauri::AppHandle) -> Result<(), String> {
    let meta_path = staged_meta_path_for_app(app)?;
    let bytes_path = staged_bytes_path_for_app(app)?;
    if meta_path.exists() {
        std::fs::remove_file(&meta_path).map_err(|e| {
            format!(
                "clearing staged update metadata '{}': {e}",
                meta_path.display()
            )
        })?;
    }
    if bytes_path.exists() {
        std::fs::remove_file(&bytes_path).map_err(|e| {
            format!(
                "clearing staged update bytes '{}': {e}",
                bytes_path.display()
            )
        })?;
    }
    Ok(())
}

fn clear_staged_update_if_current_version_is_new_enough(
    app: &tauri::AppHandle,
    current_version: &str,
) -> Result<(), String> {
    let Some(meta) = read_staged_update_meta_for_app(app)? else {
        return Ok(());
    };
    if version_is_at_or_above(current_version, &meta.version) {
        clear_staged_update_for_app(app)?;
    }
    Ok(())
}

fn has_matching_staged_update(
    app: &tauri::AppHandle,
    channel: &str,
    expected_version: &str,
    config: &DesktopNativeUpdaterConfig,
) -> Result<bool, String> {
    let Some(meta) = read_staged_update_meta_for_app(app)? else {
        return Ok(false);
    };
    if meta.version.trim() != expected_version.trim()
        || meta.target.trim() != config.target.trim()
        || meta.endpoint.trim() != config.endpoint.trim()
        || meta.channel.trim() != channel.trim()
    {
        return Ok(false);
    }
    let bytes_path = staged_bytes_path_for_app(app)?;
    let exists = bytes_path.exists();
    if !exists {
        clear_staged_update_for_app(app)?;
    }
    Ok(exists)
}

fn read_staged_update_bytes_if_matching(
    app: &tauri::AppHandle,
    channel: &str,
    expected_version: &str,
    config: &DesktopNativeUpdaterConfig,
) -> Result<Option<Vec<u8>>, String> {
    if !has_matching_staged_update(app, channel, expected_version, config)? {
        return Ok(None);
    }
    let bytes_path = staged_bytes_path_for_app(app)?;
    let bytes = std::fs::read(&bytes_path).map_err(|e| {
        format!(
            "reading staged update bytes '{}': {e}",
            bytes_path.display()
        )
    })?;
    if bytes.is_empty() {
        clear_staged_update_for_app(app)?;
        return Ok(None);
    }
    Ok(Some(bytes))
}

fn begin_update_attempt(channel: &str, current_version: &str) -> DesktopUpdateAttempt {
    DesktopUpdateAttempt {
        attempt_id: format!("desktop-updater-{}-{}", now_ms(), std::process::id()),
        channel: channel.to_string(),
        current_version: current_version.to_string(),
        target_version: None,
        started_at_ms: now_ms(),
        finished_at_ms: None,
        result: DesktopUpdateAttemptResult::InProgress,
        stages: Vec::new(),
    }
}

fn begin_attempt_stage(attempt: &mut DesktopUpdateAttempt, stage: &str) -> usize {
    attempt.stages.push(DesktopUpdateAttemptStage {
        stage: stage.to_string(),
        started_at_ms: now_ms(),
        finished_at_ms: None,
        result: DesktopUpdateAttemptResult::InProgress,
        error_code: None,
        error_message: None,
    });
    attempt.stages.len() - 1
}

fn complete_attempt_stage(attempt: &mut DesktopUpdateAttempt, index: usize) {
    if let Some(stage) = attempt.stages.get_mut(index) {
        stage.finished_at_ms = Some(now_ms());
        stage.result = DesktopUpdateAttemptResult::Succeeded;
        stage.error_code = None;
        stage.error_message = None;
    }
}

fn fail_attempt_stage(attempt: &mut DesktopUpdateAttempt, index: usize, code: &str, message: &str) {
    if let Some(stage) = attempt.stages.get_mut(index) {
        stage.finished_at_ms = Some(now_ms());
        stage.result = DesktopUpdateAttemptResult::Failed;
        stage.error_code = Some(code.to_string());
        stage.error_message = Some(message.to_string());
    }
}

fn mark_attempt_succeeded(attempt: &mut DesktopUpdateAttempt) {
    attempt.finished_at_ms = Some(now_ms());
    attempt.result = DesktopUpdateAttemptResult::Succeeded;
}

fn mark_attempt_failed(attempt: &mut DesktopUpdateAttempt) {
    attempt.finished_at_ms = Some(now_ms());
    attempt.result = DesktopUpdateAttemptResult::Failed;
}

fn persist_attempt_success_best_effort(app: &tauri::AppHandle, attempt: &mut DesktopUpdateAttempt) {
    mark_attempt_succeeded(attempt);
    if let Err(write_err) = write_last_attempt_for_app(app, attempt) {
        eprintln!("warn: failed to persist updater success attempt: {write_err}");
    }
}

fn persist_attempt_failure_best_effort(
    app: &tauri::AppHandle,
    attempt: &mut DesktopUpdateAttempt,
    err: String,
) -> String {
    mark_attempt_failed(attempt);
    if let Err(write_err) = write_last_attempt_for_app(app, attempt) {
        eprintln!("warn: failed to persist updater failure attempt: {write_err}");
    }
    err
}

fn write_last_attempt_for_app(
    app: &tauri::AppHandle,
    attempt: &DesktopUpdateAttempt,
) -> Result<(), String> {
    let path = last_attempt_path_for_app(app)?;
    let encoded = serde_json::to_string_pretty(attempt)
        .map_err(|e| format!("encoding desktop updater attempt: {e}"))?;
    std::fs::write(&path, format!("{encoded}\n"))
        .map_err(|e| format!("writing desktop updater attempt '{}': {e}", path.display()))
}

fn read_last_attempt_for_app(
    app: &tauri::AppHandle,
) -> Result<Option<DesktopUpdateAttempt>, String> {
    let path = last_attempt_path_for_app(app)?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("reading desktop updater attempt '{}': {e}", path.display()))?;
    let parsed = serde_json::from_str::<DesktopUpdateAttempt>(&raw)
        .map_err(|e| format!("parsing desktop updater attempt '{}': {e}", path.display()))?;
    Ok(Some(parsed))
}

fn last_failed_stage_message(attempt: &DesktopUpdateAttempt) -> Option<&str> {
    if attempt.result != DesktopUpdateAttemptResult::Failed {
        return None;
    }
    attempt
        .stages
        .iter()
        .rev()
        .find_map(|stage| stage.error_message.as_deref())
}

async fn stage_update_in_background(app: tauri::AppHandle, channel: &str) -> Result<(), String> {
    let current_version = app.package_info().version.to_string();
    let mut attempt = begin_update_attempt(channel, &current_version);

    let config = resolve_native_updater_config(channel)?;
    let Some(pubkey) = config.pubkey.as_deref() else {
        clear_staged_update_for_app(&app)?;
        persist_attempt_success_best_effort(&app, &mut attempt);
        return Ok(());
    };
    let endpoint_url = endpoint_with_download_id(&config.endpoint, None)?;
    let build_stage = begin_attempt_stage(&mut attempt, "build");
    let updater = app
        .updater_builder()
        .target(config.target.clone())
        .pubkey(pubkey)
        .endpoints(vec![endpoint_url])
        .map_err(|e| {
            let err = updater_stage_error("build", e);
            fail_attempt_stage(&mut attempt, build_stage, "build", &err);
            persist_attempt_failure_best_effort(&app, &mut attempt, err)
        })?
        .build()
        .map_err(|e| {
            let err = updater_stage_error("build", e);
            fail_attempt_stage(&mut attempt, build_stage, "build", &err);
            persist_attempt_failure_best_effort(&app, &mut attempt, err)
        })?;
    complete_attempt_stage(&mut attempt, build_stage);

    let check_stage = begin_attempt_stage(&mut attempt, "check");
    let Some(update) = updater.check().await.map_err(|e| {
        let err = updater_stage_error("check", e);
        fail_attempt_stage(&mut attempt, check_stage, "check", &err);
        persist_attempt_failure_best_effort(&app, &mut attempt, err)
    })?
    else {
        complete_attempt_stage(&mut attempt, check_stage);
        clear_staged_update_for_app(&app)?;
        persist_attempt_success_best_effort(&app, &mut attempt);
        return Ok(());
    };
    complete_attempt_stage(&mut attempt, check_stage);
    let latest_version = update.version.clone();
    attempt.target_version = Some(latest_version.clone());

    let verify_stage = begin_attempt_stage(&mut attempt, "verify");
    if !version_is_strictly_newer(&latest_version, &current_version) {
        complete_attempt_stage(&mut attempt, verify_stage);
        clear_staged_update_for_app(&app)?;
        persist_attempt_success_best_effort(&app, &mut attempt);
        return Ok(());
    }
    complete_attempt_stage(&mut attempt, verify_stage);

    let download_stage = begin_attempt_stage(&mut attempt, "download");
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| {
        let err = updater_stage_error("download", e);
        fail_attempt_stage(&mut attempt, download_stage, "download", &err);
        persist_attempt_failure_best_effort(&app, &mut attempt, err)
    })?;
    complete_attempt_stage(&mut attempt, download_stage);

    let marker_stage = begin_attempt_stage(&mut attempt, "marker");
    let meta = DesktopStagedUpdateMeta {
        version: latest_version,
        target: config.target,
        endpoint: config.endpoint,
        channel: channel.to_string(),
        downloaded_at_ms: now_ms(),
        size_bytes: bytes.len(),
    };
    write_staged_update_for_app(&app, &meta, &bytes).map_err(|e| {
        let err = updater_stage_error("marker_write", e);
        fail_attempt_stage(&mut attempt, marker_stage, "marker_write", &err);
        persist_attempt_failure_best_effort(&app, &mut attempt, err)
    })?;
    complete_attempt_stage(&mut attempt, marker_stage);
    persist_attempt_success_best_effort(&app, &mut attempt);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedVersion {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<String>,
}

fn parse_semver_like(value: &str) -> Option<ParsedVersion> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let (without_build, _) = normalized.split_once('+').unwrap_or((normalized, ""));
    let (core, pre) = without_build
        .split_once('-')
        .map(|(lhs, rhs)| (lhs, Some(rhs.to_string())))
        .unwrap_or((without_build, None));
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts
        .next()
        .map(|v| v.parse::<u64>().ok())
        .unwrap_or(Some(0))?;
    let patch = parts
        .next()
        .map(|v| v.parse::<u64>().ok())
        .unwrap_or(Some(0))?;
    if parts.next().is_some() {
        return None;
    }
    Some(ParsedVersion {
        major,
        minor,
        patch,
        pre,
    })
}

fn compare_prerelease_segments(lhs: &str, rhs: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    for (left_seg, right_seg) in lhs.split('.').zip(rhs.split('.')) {
        let left_num = left_seg.parse::<u64>().ok();
        let right_num = right_seg.parse::<u64>().ok();
        let ord = match (left_num, right_num) {
            (Some(l), Some(r)) => l.cmp(&r),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => left_seg.cmp(right_seg),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    lhs.split('.').count().cmp(&rhs.split('.').count())
}

fn compare_semver_like(lhs: &ParsedVersion, rhs: &ParsedVersion) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let core_cmp = (lhs.major, lhs.minor, lhs.patch).cmp(&(rhs.major, rhs.minor, rhs.patch));
    if core_cmp != Ordering::Equal {
        return core_cmp;
    }
    match (&lhs.pre, &rhs.pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(l), Some(r)) => compare_prerelease_segments(l, r),
    }
}

fn version_is_strictly_newer(candidate: &str, current: &str) -> bool {
    let Some(candidate_ver) = parse_semver_like(candidate) else {
        return false;
    };
    let Some(current_ver) = parse_semver_like(current) else {
        return false;
    };
    compare_semver_like(&candidate_ver, &current_ver).is_gt()
}

fn version_is_at_or_above(current: &str, required: &str) -> bool {
    match (parse_semver_like(current), parse_semver_like(required)) {
        (Some(current_ver), Some(required_ver)) => {
            !compare_semver_like(&current_ver, &required_ver).is_lt()
        }
        _ => current.trim() == required.trim(),
    }
}

fn normalize_latest_version(
    current_version: &str,
    latest_from_feed: Option<&str>,
    pending_restart_version: Option<&str>,
) -> Option<String> {
    if let Some(pending) = pending_restart_version {
        let trimmed = pending.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    let candidate = latest_from_feed?.trim();
    if candidate.is_empty() {
        return None;
    }
    if version_is_strictly_newer(candidate, current_version) {
        return Some(candidate.to_string());
    }
    None
}

#[derive(Debug, Deserialize, Serialize)]
struct RestartMarker {
    version: String,
}

fn restart_marker_path_for_app(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let mut dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolving app_data_dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating app_data_dir: {e}"))?;
    dir.push(RESTART_MARKER_FILENAME);
    Ok(dir)
}

fn read_restart_marker(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "reading desktop updater restart marker '{}': {e}",
            path.display()
        )
    })?;
    let parsed: RestartMarker = serde_json::from_str(&raw).map_err(|e| {
        format!(
            "parsing desktop updater restart marker '{}': {e}",
            path.display()
        )
    })?;
    let trimmed = parsed.version.trim();
    if trimmed.is_empty() {
        clear_restart_marker(path)?;
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

fn write_restart_marker(path: &Path, version: &str) -> Result<(), String> {
    let payload = RestartMarker {
        version: version.trim().to_string(),
    };
    let encoded = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("encoding desktop updater restart marker: {e}"))?;
    std::fs::write(path, format!("{encoded}\n")).map_err(|e| {
        format!(
            "writing desktop updater restart marker '{}': {e}",
            path.display()
        )
    })
}

fn clear_restart_marker(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(path).map_err(|e| {
        format!(
            "clearing desktop updater restart marker '{}': {e}",
            path.display()
        )
    })
}

fn reconcile_restart_marker(path: &Path, current_version: &str) -> Result<Option<String>, String> {
    let marker = read_restart_marker(path)?;
    let Some(marker_version) = marker else {
        return Ok(None);
    };
    if version_is_at_or_above(current_version, &marker_version) {
        clear_restart_marker(path)?;
        return Ok(None);
    }
    Ok(Some(marker_version))
}

fn reconcile_restart_marker_for_app(
    app: &tauri::AppHandle,
    current_version: &str,
) -> Result<Option<String>, String> {
    let path = restart_marker_path_for_app(app)?;
    reconcile_restart_marker(&path, current_version)
}

fn write_restart_marker_for_app(app: &tauri::AppHandle, version: &str) -> Result<(), String> {
    let path = restart_marker_path_for_app(app)?;
    write_restart_marker(&path, version)
}

fn normalize_download_id(raw: Option<&str>) -> Option<String> {
    let candidate = raw?.trim();
    if candidate.is_empty() || candidate.len() > 64 {
        return None;
    }
    if !candidate
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':'))
    {
        return None;
    }
    Some(candidate.to_string())
}

fn endpoint_with_download_id(endpoint: &str, download_id: Option<&str>) -> Result<Url, String> {
    let mut parsed = Url::parse(endpoint).map_err(|e| format!("invalid update endpoint: {e}"))?;
    if let Some(download_id) = download_id {
        parsed
            .query_pairs_mut()
            .append_pair("ctx_download_id", download_id);
    }
    Ok(parsed)
}

fn resolve_native_updater_config(channel: &str) -> Result<DesktopNativeUpdaterConfig, String> {
    let target = desktop_platform_key()?;
    let base_url = default_download_base_url();
    let endpoint_default = format!(
        "{}/releases/{}/latest-tauri.json",
        base_url.trim_end_matches('/'),
        channel
    );
    let endpoint = std::env::var("CTX_DESKTOP_UPDATER_ENDPOINT")
        .ok()
        .and_then(|raw| expand_updater_endpoint_template(&raw, channel))
        .unwrap_or(endpoint_default);
    let pubkey = resolve_updater_pubkey(
        std::env::var("CTX_DESKTOP_UPDATER_PUBKEY").ok(),
        option_env!("CTX_DESKTOP_UPDATER_PUBKEY"),
    );
    Ok(DesktopNativeUpdaterConfig {
        target: target.to_string(),
        endpoint,
        pubkey,
    })
}

fn normalize_nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn updater_stage_error(stage: &str, err: impl std::fmt::Display) -> String {
    format!("native updater {stage} failed: {err}")
}

fn resolve_updater_pubkey(
    runtime_value: Option<String>,
    build_value: Option<&str>,
) -> Option<String> {
    let runtime = runtime_value
        .as_deref()
        .and_then(normalize_nonempty)
        .and_then(normalize_updater_pubkey);
    if runtime.is_some() {
        return runtime;
    }
    build_value
        .and_then(normalize_nonempty)
        .and_then(normalize_updater_pubkey)
}

fn normalize_updater_pubkey(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(normalized_plain) = normalize_minisign_pubkey_text(trimmed) {
        return Some(BASE64_STANDARD.encode(normalized_plain.as_bytes()));
    }
    let compact: String = trimmed
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    if let Some(decoded_plain) = decode_base64_minisign_pubkey(&compact) {
        return Some(BASE64_STANDARD.encode(decoded_plain.as_bytes()));
    }
    None
}

fn decode_base64_minisign_pubkey(encoded: &str) -> Option<String> {
    let decoded_bytes = BASE64_STANDARD.decode(encoded.as_bytes()).ok()?;
    let decoded_text = String::from_utf8(decoded_bytes).ok()?;
    normalize_minisign_pubkey_text(&decoded_text)
}

fn normalize_minisign_pubkey_text(raw: &str) -> Option<String> {
    let normalized = raw.replace("\r\n", "\n");
    let mut lines = normalized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let header = lines.next()?;
    if !header.starts_with("untrusted comment: minisign public key:") {
        return None;
    }
    let key_line = lines.next()?;
    if key_line.is_empty() || lines.next().is_some() {
        return None;
    }
    Some(format!("{header}\n{key_line}\n"))
}

fn default_download_base_url() -> String {
    std::env::var("CTX_DOWNLOAD_BASE_URL")
        .unwrap_or_else(|_| "https://api.ctx.rs/functions/v1".to_string())
}

fn expand_updater_endpoint_template(raw: &str, channel: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.replace("{channel}", channel))
}

fn desktop_platform_key() -> Result<&'static str, String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("linux", "x86_64") => Ok("linux-x64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("macos", "x86_64") => Ok("macos-x64"),
        ("macos", "aarch64") => Ok("macos-arm64"),
        ("windows", "x86_64") => Ok("windows-x64"),
        _ => Err(format!(
            "unsupported platform for desktop updater: {os}/{arch}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        path.push(format!(
            "ctx-desktop-updater-{label}-{}-{now}.json",
            std::process::id()
        ));
        path
    }

    #[test]
    fn desktop_platform_key_is_known_for_current_target() {
        let key = desktop_platform_key();
        assert!(
            key.is_ok(),
            "current target should map to known update platform key: {key:?}"
        );
    }

    #[test]
    fn resolve_native_updater_config_uses_base_defaults() {
        let cfg = resolve_native_updater_config("stable").expect("config should resolve");
        assert!(
            cfg.endpoint.ends_with("/releases/stable/latest-tauri.json"),
            "unexpected endpoint: {}",
            cfg.endpoint
        );
    }

    #[test]
    fn expand_updater_endpoint_template_replaces_channel_placeholder() {
        let cfg = expand_updater_endpoint_template(
            "https://example.test/releases/{channel}/latest-tauri.json",
            "rc-2026.02.17",
        )
        .expect("template should expand");
        assert_eq!(
            cfg,
            "https://example.test/releases/rc-2026.02.17/latest-tauri.json"
        );
    }

    #[test]
    fn endpoint_with_download_id_appends_query_param() {
        let url = endpoint_with_download_id(
            "https://example.test/releases/stable/latest-tauri.json",
            Some("abc-123"),
        )
        .expect("endpoint should parse");
        assert_eq!(
            url.as_str(),
            "https://example.test/releases/stable/latest-tauri.json?ctx_download_id=abc-123"
        );
    }

    #[test]
    fn normalize_download_id_rejects_invalid_chars() {
        let value = normalize_download_id(Some("abc def"));
        assert!(value.is_none());
    }

    #[test]
    fn resolve_updater_pubkey_prefers_runtime_value() {
        let runtime_raw =
            "untrusted comment: minisign public key: 0D503F73CDD77B9C\nRWSce9fNcz9QDfv7dghgOH/dIA0Txkgk8rB86J5s6I15e+NkpWjU3CFs\n";
        let build_raw =
            "untrusted comment: minisign public key: ABCDEF0123456789\nRWSce9fNcz9QAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n";
        let key = resolve_updater_pubkey(Some(runtime_raw.to_string()), Some(build_raw))
            .expect("resolved key");
        let decoded = String::from_utf8(
            BASE64_STANDARD
                .decode(key.as_bytes())
                .expect("runtime key should decode as base64"),
        )
        .expect("runtime key should decode as utf8");
        assert_eq!(decoded, runtime_raw);
    }

    #[test]
    fn resolve_updater_pubkey_decodes_base64_minisign_key() {
        let raw =
            "untrusted comment: minisign public key: 0D503F73CDD77B9C\nRWSce9fNcz9QDfv7dghgOH/dIA0Txkgk8rB86J5s6I15e+NkpWjU3CFs\n";
        let encoded = BASE64_STANDARD.encode(raw.as_bytes());
        let key = resolve_updater_pubkey(Some(encoded), None).expect("resolved key");
        assert_eq!(key, BASE64_STANDARD.encode(raw.as_bytes()));
    }

    #[test]
    fn resolve_updater_pubkey_falls_back_to_build_value_when_runtime_invalid() {
        let build_raw =
            "untrusted comment: minisign public key: 0D503F73CDD77B9C\nRWSce9fNcz9QDfv7dghgOH/dIA0Txkgk8rB86J5s6I15e+NkpWjU3CFs\n";
        let key = resolve_updater_pubkey(Some("not-a-valid-key".to_string()), Some(build_raw))
            .expect("resolved key");
        assert_eq!(key, BASE64_STANDARD.encode(build_raw.as_bytes()));
    }

    #[test]
    fn resolve_updater_pubkey_returns_none_when_both_sources_empty() {
        assert!(resolve_updater_pubkey(Some("".to_string()), Some("  ")).is_none());
    }

    #[test]
    fn resolve_updater_pubkey_rejects_invalid_values() {
        assert!(resolve_updater_pubkey(Some("invalid".to_string()), None).is_none());
        assert!(resolve_updater_pubkey(Some("   ".to_string()), None).is_none());
    }

    #[test]
    fn tauri_manifest_parser_accepts_absolute_updater_urls() {
        let manifest = r#"{
          "version":"1.2.3",
          "notes":"ctx 1.2.3",
          "pub_date":"2026-03-03T00:00:00Z",
          "platforms":{
            "macos-arm64":{
              "url":"https://api.ctx.rs/functions/v1/download/stable/1.2.3/ctx_1.2.3_macos-arm64_updater.app.tar.gz",
              "signature":"sig"
            }
          }
        }"#;
        let parsed = serde_json::from_str::<tauri_plugin_updater::RemoteRelease>(manifest);
        assert!(
            parsed.is_ok(),
            "absolute updater URLs should parse: {parsed:?}"
        );
    }

    #[test]
    fn tauri_manifest_parser_rejects_relative_updater_urls() {
        let manifest = r#"{
          "version":"1.2.3",
          "notes":"ctx 1.2.3",
          "pub_date":"2026-03-03T00:00:00Z",
          "platforms":{
            "macos-arm64":{
              "url":"/download/stable/1.2.3/ctx_1.2.3_macos-arm64_updater.app.tar.gz",
              "signature":"sig"
            }
          }
        }"#;
        let parsed = serde_json::from_str::<tauri_plugin_updater::RemoteRelease>(manifest);
        assert!(parsed.is_err(), "relative updater URLs must be rejected");
    }

    #[test]
    fn version_is_strictly_newer_respects_semver_ordering() {
        assert!(version_is_strictly_newer("1.2.0", "1.1.9"));
        assert!(!version_is_strictly_newer("1.2.0", "1.2.0"));
        assert!(!version_is_strictly_newer("1.2.0", "1.2.1"));
    }

    #[test]
    fn normalize_latest_version_ignores_equal_or_older_candidates() {
        assert_eq!(normalize_latest_version("1.2.3", Some("1.2.3"), None), None);
        assert_eq!(normalize_latest_version("1.2.3", Some("1.2.2"), None), None);
        assert_eq!(
            normalize_latest_version("1.2.3", Some("1.2.4"), None),
            Some("1.2.4".to_string())
        );
    }

    #[test]
    fn normalize_latest_version_prefers_pending_restart_version() {
        assert_eq!(
            normalize_latest_version("1.2.3", Some("1.3.0"), Some("1.2.9")),
            Some("1.2.9".to_string())
        );
    }

    #[test]
    fn reconcile_restart_marker_clears_when_current_is_new_enough() {
        let path = temp_path("marker-clear");
        write_restart_marker(&path, "2.0.0").expect("write marker");
        let marker = reconcile_restart_marker(&path, "2.0.0").expect("reconcile");
        assert!(marker.is_none());
        assert!(!path.exists(), "marker file should be removed");
    }

    #[test]
    fn reconcile_restart_marker_keeps_pending_when_current_is_older() {
        let path = temp_path("marker-pending");
        write_restart_marker(&path, "2.0.0").expect("write marker");
        let marker = reconcile_restart_marker(&path, "1.9.9").expect("reconcile");
        assert_eq!(marker.as_deref(), Some("2.0.0"));
        assert!(
            path.exists(),
            "marker file should remain while restart is pending"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn desktop_update_phase_strings_are_stable() {
        assert_eq!(DesktopAppUpdatePhase::Idle.as_str(), "idle");
        assert_eq!(DesktopAppUpdatePhase::Staging.as_str(), "staging");
        assert_eq!(DesktopAppUpdatePhase::StagedReady.as_str(), "staged_ready");
        assert_eq!(
            DesktopAppUpdatePhase::RestartRequired.as_str(),
            "restart_required"
        );
        assert_eq!(DesktopAppUpdatePhase::Failed.as_str(), "failed");
    }

    #[test]
    fn last_failed_stage_message_reports_latest_failure() {
        let attempt = DesktopUpdateAttempt {
            attempt_id: "attempt".to_string(),
            channel: "stable".to_string(),
            current_version: "0.4.9".to_string(),
            target_version: Some("0.4.10".to_string()),
            started_at_ms: 1,
            finished_at_ms: Some(2),
            result: DesktopUpdateAttemptResult::Failed,
            stages: vec![
                DesktopUpdateAttemptStage {
                    stage: "check".to_string(),
                    started_at_ms: 1,
                    finished_at_ms: Some(1),
                    result: DesktopUpdateAttemptResult::Failed,
                    error_code: Some("check".to_string()),
                    error_message: Some("first".to_string()),
                },
                DesktopUpdateAttemptStage {
                    stage: "install".to_string(),
                    started_at_ms: 2,
                    finished_at_ms: Some(2),
                    result: DesktopUpdateAttemptResult::Failed,
                    error_code: Some("install".to_string()),
                    error_message: Some("latest".to_string()),
                },
            ],
        };
        assert_eq!(last_failed_stage_message(&attempt), Some("latest"));
    }

    #[test]
    fn desktop_update_attempt_json_round_trip() {
        let attempt = DesktopUpdateAttempt {
            attempt_id: "attempt-123".to_string(),
            channel: "stable".to_string(),
            current_version: "0.4.9".to_string(),
            target_version: Some("0.4.10".to_string()),
            started_at_ms: 100,
            finished_at_ms: Some(200),
            result: DesktopUpdateAttemptResult::Succeeded,
            stages: vec![DesktopUpdateAttemptStage {
                stage: "download".to_string(),
                started_at_ms: 120,
                finished_at_ms: Some(180),
                result: DesktopUpdateAttemptResult::Succeeded,
                error_code: None,
                error_message: None,
            }],
        };
        let encoded = serde_json::to_string(&attempt).expect("encode attempt");
        let decoded: DesktopUpdateAttempt = serde_json::from_str(&encoded).expect("decode attempt");
        assert_eq!(decoded.attempt_id, "attempt-123");
        assert_eq!(decoded.target_version.as_deref(), Some("0.4.10"));
        assert_eq!(decoded.stages.len(), 1);
        assert_eq!(decoded.stages[0].stage, "download");
    }
}
