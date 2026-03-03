use super::*;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use tauri_plugin_updater::UpdaterExt;
use url::Url;

const RESTART_MARKER_FILENAME: &str = "desktop_update_restart_required.json";

#[derive(Debug, Deserialize)]
pub(super) struct DesktopAppUpdateCheckReq {
    #[serde(default)]
    channel: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DesktopAppUpdateCheckResp {
    configured: bool,
    available: bool,
    current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_version: Option<String>,
    target: String,
    endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DesktopAppUpdateStateResp {
    configured: bool,
    available: bool,
    restart_required: bool,
    current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_version: Option<String>,
    target: String,
    endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
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

#[tauri::command]
pub(super) async fn desktop_check_app_update(
    app: tauri::AppHandle,
    req: DesktopAppUpdateCheckReq,
) -> Result<DesktopAppUpdateCheckResp, String> {
    let state = desktop_get_app_update_state(app, req).await?;
    Ok(DesktopAppUpdateCheckResp {
        configured: state.configured,
        available: state.available,
        current_version: state.current_version,
        latest_version: state.latest_version,
        target: state.target,
        endpoint: state.endpoint,
        message: state.message,
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
            message: "Desktop update already installed. Relaunch the app to complete the update."
                .to_string(),
        });
    }

    if !pre_state.configured {
        return Err(
            "native updater is not configured (missing CTX_DESKTOP_UPDATER_PUBKEY)".to_string(),
        );
    }

    let config = resolve_native_updater_config(&channel)?;
    let pubkey = config.pubkey.as_deref().ok_or_else(|| {
        "native updater is not configured (missing CTX_DESKTOP_UPDATER_PUBKEY)".to_string()
    })?;
    let endpoint_url = endpoint_with_download_id(&config.endpoint, download_id.as_deref())?;
    let updater = app
        .updater_builder()
        .target(config.target.clone())
        .pubkey(pubkey)
        .endpoints(vec![endpoint_url])
        .map_err(to_err)?
        .build()
        .map_err(to_err)?;
    let Some(update) = updater.check().await.map_err(to_err)? else {
        return Ok(DesktopAppUpdateApplyResp {
            applied: false,
            needs_restart: false,
            up_to_date: true,
            latest_version: None,
            message: "No desktop app update is currently available.".to_string(),
        });
    };
    let latest_version = update.version.clone();
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(to_err)?;
    write_restart_marker_for_app(&app, &latest_version)?;

    Ok(DesktopAppUpdateApplyResp {
        applied: true,
        needs_restart: true,
        up_to_date: false,
        latest_version: Some(latest_version),
        message: "Desktop update installed. Relaunch the app to complete the update.".to_string(),
    })
}

#[tauri::command]
pub(super) fn desktop_restart_app(app: tauri::AppHandle) -> Result<DesktopAppRestartResp, String> {
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
    let pending_restart_version = reconcile_restart_marker_for_app(app, &current_version)?;
    let restart_required = pending_restart_version.is_some();
    let message = if config.pubkey.is_none() {
        Some("Native updater is not configured (missing CTX_DESKTOP_UPDATER_PUBKEY).".to_string())
    } else {
        None
    };
    let Some(pubkey) = config.pubkey.as_deref() else {
        return Ok(DesktopAppUpdateStateResp {
            configured: false,
            available: false,
            restart_required,
            current_version,
            latest_version: pending_restart_version,
            target: config.target,
            endpoint: config.endpoint,
            message,
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
    let update = updater.check().await.map_err(to_err)?;
    let raw_latest_version = update.as_ref().map(|v| v.version.clone());
    let latest_from_feed = normalize_latest_version(
        &current_version,
        raw_latest_version.as_deref(),
        pending_restart_version.as_deref(),
    );
    let available = raw_latest_version
        .as_deref()
        .map(|latest| version_is_strictly_newer(latest, &current_version))
        .unwrap_or(false)
        && !restart_required;
    Ok(DesktopAppUpdateStateResp {
        configured: true,
        available,
        restart_required,
        current_version,
        latest_version: latest_from_feed,
        target: config.target,
        endpoint: config.endpoint,
        message,
    })
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

fn resolve_updater_pubkey(
    runtime_value: Option<String>,
    build_value: Option<&str>,
) -> Option<String> {
    runtime_value
        .as_deref()
        .and_then(normalize_nonempty)
        .or_else(|| build_value.and_then(normalize_nonempty))
        .and_then(normalize_updater_pubkey)
}

fn normalize_updater_pubkey(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("untrusted comment: minisign public key:") {
        return Some(trimmed.to_string());
    }
    if let Some(decoded) = decode_base64_minisign_pubkey(trimmed) {
        return Some(decoded);
    }
    Some(trimmed.to_string())
}

fn decode_base64_minisign_pubkey(encoded: &str) -> Option<String> {
    let decoded_bytes = BASE64_STANDARD.decode(encoded.as_bytes()).ok()?;
    let decoded_text = String::from_utf8(decoded_bytes).ok()?;
    let normalized = decoded_text.replace("\r\n", "\n").trim().to_string();
    if normalized.starts_with("untrusted comment: minisign public key:")
        && normalized.lines().nth(1).is_some()
    {
        return Some(normalized);
    }
    None
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
        let key = resolve_updater_pubkey(Some(" runtime-key ".to_string()), Some("build-key"))
            .expect("resolved key");
        assert_eq!(key, "runtime-key");
    }

    #[test]
    fn resolve_updater_pubkey_decodes_base64_minisign_key() {
        let raw =
            "untrusted comment: minisign public key: 0D503F73CDD77B9C\nRWSce9fNcz9QDfv7dghgOH/dIA0Txkgk8rB86J5s6I15e+NkpWjU3CFs\n";
        let encoded = BASE64_STANDARD.encode(raw.as_bytes());
        let key = resolve_updater_pubkey(Some(encoded), None).expect("resolved key");
        assert_eq!(key, raw.trim());
    }

    #[test]
    fn resolve_updater_pubkey_falls_back_to_build_value() {
        let key = resolve_updater_pubkey(Some("  ".to_string()), Some(" build-key "))
            .expect("resolved key");
        assert_eq!(key, "build-key");
    }

    #[test]
    fn resolve_updater_pubkey_returns_none_when_both_sources_empty() {
        assert!(resolve_updater_pubkey(Some("".to_string()), Some("  ")).is_none());
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
        assert!(parsed.is_ok(), "absolute updater URLs should parse: {parsed:?}");
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
        assert_eq!(
            normalize_latest_version("1.2.3", Some("1.2.3"), None),
            None
        );
        assert_eq!(
            normalize_latest_version("1.2.3", Some("1.2.2"), None),
            None
        );
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
        assert!(path.exists(), "marker file should remain while restart is pending");
        let _ = std::fs::remove_file(path);
    }
}
