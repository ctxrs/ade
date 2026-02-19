use super::*;

use tauri_plugin_updater::UpdaterExt;
use url::Url;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_version: Option<String>,
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
    let channel = desktop_ssh::normalize_update_channel(req.channel.as_deref())?;
    let current_version = app.package_info().version.to_string();
    let config = resolve_native_updater_config(&channel)?;
    let message = if config.pubkey.is_none() {
        Some("Native updater is not configured (missing CTX_DESKTOP_UPDATER_PUBKEY).".to_string())
    } else {
        None
    };

    let Some(pubkey) = config.pubkey.as_deref() else {
        return Ok(DesktopAppUpdateCheckResp {
            configured: false,
            available: false,
            current_version,
            latest_version: None,
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
    let latest_version = update.as_ref().map(|v| v.version.clone());
    Ok(DesktopAppUpdateCheckResp {
        configured: true,
        available: update.is_some(),
        current_version,
        latest_version,
        target: config.target,
        endpoint: config.endpoint,
        message,
    })
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
            latest_version: None,
            message: "No desktop app update is currently available.".to_string(),
        });
    };
    let latest_version = update.version.clone();
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(to_err)?;

    Ok(DesktopAppUpdateApplyResp {
        applied: true,
        needs_restart: true,
        latest_version: Some(latest_version),
        message: "Desktop update installed. Relaunch the app to complete the update.".to_string(),
    })
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
    let pubkey = std::env::var("CTX_DESKTOP_UPDATER_PUBKEY")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    Ok(DesktopNativeUpdaterConfig {
        target: target.to_string(),
        endpoint,
        pubkey,
    })
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
}
