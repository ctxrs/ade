use super::*;
use crate::api::MobileAuthContext;
use axum::Extension;
use ctx_core::provider_policy::CODEX_APP_SERVER_ARGS;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

mod auth_url;
mod browser;
mod claude;
mod codex;
mod kimi;
mod mistral;

pub(super) use auth_url::{
    auth_url_looks_complete, extract_auth_url, normalize_claude_login_line,
    read_trailing_claude_login_lines,
};
pub(crate) use browser::{
    get_amp_login, get_gemini_login, get_qwen_login, start_amp_login, start_gemini_login,
    start_qwen_login,
};
#[cfg(test)]
pub(super) use claude::resolve_claude_login_runtime_from_config;
pub(crate) use claude::{get_claude_login, start_claude_login};
pub(crate) use codex::{complete_codex_login, get_codex_login, start_codex_login};
pub(crate) use kimi::{get_kimi_login, start_kimi_login};
pub(crate) use mistral::{get_mistral_login, start_mistral_login};

const CODEX_LOGIN_RPC_TIMEOUT: Duration = Duration::from_secs(30);
const CLAUDE_LOGIN_URL_WAIT: Duration = Duration::from_secs(20);
const GEMINI_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const GEMINI_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const QWEN_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const QWEN_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const AMP_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const AMP_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const MISTRAL_LOGIN_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const MISTRAL_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const QWEN_OAUTH_AUTH_METHOD_ID: &str = "qwen-oauth";
const AMP_BROWSER_AUTH_METHOD_ID: &str = "amp_browser_login";

pub(super) fn reject_mobile_auth(
    mobile_auth: Option<Extension<MobileAuthContext>>,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    if mobile_auth.is_some() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "desktop auth required".to_string(),
            }),
        ));
    }
    Ok(())
}

fn is_loopback_host(value: &str) -> bool {
    let host = value.trim().to_ascii_lowercase();
    if host == "localhost" {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    false
}

fn normalized_host(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(super) fn expected_callback_from_auth_url(auth_url: &str) -> Option<String> {
    let parsed = Url::parse(auth_url).ok()?;
    let redirect = parsed
        .query_pairs()
        .find_map(|(key, value)| (key == "redirect_uri").then_some(value.into_owned()))?;
    let callback = Url::parse(&redirect).ok()?;
    let host = callback.host_str()?;
    if !is_loopback_host(host) {
        return None;
    }
    Some(callback.to_string())
}

pub(super) fn validate_callback_url(
    callback_url: &str,
    expected_callback_url: Option<&str>,
) -> anyhow::Result<()> {
    let callback = Url::parse(callback_url)
        .with_context(|| format!("invalid callback_url: {callback_url}"))?;
    if callback.scheme() != "http" {
        anyhow::bail!("callback_url must use http scheme");
    }
    let host = callback
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("callback_url must include host"))?;
    if !is_loopback_host(host) {
        anyhow::bail!("callback_url host must be loopback");
    }
    if callback.port().is_none() {
        anyhow::bail!("callback_url must include explicit port");
    }
    if !callback.path().starts_with("/auth/callback") {
        anyhow::bail!("callback_url path must start with /auth/callback");
    }
    if callback.query().is_none() {
        anyhow::bail!("callback_url must include query parameters");
    }

    if let Some(expected_raw) = expected_callback_url {
        let expected = Url::parse(expected_raw)
            .with_context(|| format!("invalid expected callback URL: {expected_raw}"))?;
        let expected_host = expected
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("expected callback URL must include host"))?;
        let same_host = normalized_host(host) == normalized_host(expected_host);
        let same_loopback = is_loopback_host(host) && is_loopback_host(expected_host);
        if !same_host && !same_loopback {
            anyhow::bail!("callback_url host mismatch");
        }
        if callback.scheme() != expected.scheme() {
            anyhow::bail!("callback_url scheme mismatch");
        }
        if callback.port() != expected.port() {
            anyhow::bail!("callback_url port mismatch");
        }
        if callback.path() != expected.path() {
            anyhow::bail!("callback_url path mismatch");
        }
    }
    Ok(())
}

fn gemini_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_GEMINI_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(GEMINI_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn qwen_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_QWEN_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(QWEN_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn amp_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_AMP_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(AMP_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn mistral_login_timeout() -> Duration {
    let seconds = std::env::var("CTX_MISTRAL_LOGIN_TIMEOUT_SECS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(MISTRAL_LOGIN_TIMEOUT_DEFAULT.as_secs());
    Duration::from_secs(seconds)
}

fn first_email_from_google_accounts(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            let email = map
                .get("email")
                .or_else(|| map.get("accountEmail"))
                .or_else(|| map.get("account_email"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|candidate| !candidate.is_empty())
                .map(ToString::to_string);
            email.or_else(|| map.values().find_map(first_email_from_google_accounts))
        }
        serde_json::Value::Array(values) => {
            values.iter().find_map(first_email_from_google_accounts)
        }
        _ => None,
    }
}

fn first_email_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            let direct = map
                .get("email")
                .or_else(|| map.get("accountEmail"))
                .or_else(|| map.get("account_email"))
                .or_else(|| map.get("user_email"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|candidate| !candidate.is_empty())
                .map(ToString::to_string);
            direct.or_else(|| map.values().find_map(first_email_from_value))
        }
        serde_json::Value::Array(values) => values.iter().find_map(first_email_from_value),
        _ => None,
    }
}

pub(super) fn extract_auth_url_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(raw) => {
            let trimmed = raw.trim();
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                return Some(trimmed.to_string());
            }
            extract_auth_url(trimmed)
        }
        serde_json::Value::Object(map) => {
            let direct = [
                "auth_url",
                "authUrl",
                "url",
                "login_url",
                "loginUrl",
                "authorize_url",
            ]
            .into_iter()
            .find_map(|key| map.get(key))
            .and_then(extract_auth_url_from_value);
            direct.or_else(|| map.values().find_map(extract_auth_url_from_value))
        }
        serde_json::Value::Array(values) => values.iter().find_map(extract_auth_url_from_value),
        _ => None,
    }
}

fn auth_notice_code(payload: &serde_json::Value) -> &str {
    payload
        .get("code")
        .or_else(|| payload.get("kind"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

fn is_auth_success_notice_code(code: &str) -> bool {
    matches!(
        code,
        "auth_complete" | "auth_completed" | "auth_success" | "authenticated"
    )
}

fn is_auth_failure_notice_code(code: &str) -> bool {
    matches!(
        code,
        "auth_failed" | "auth_error" | "provider_session_ref_claim_failed"
    )
}

pub(super) async fn resolve_runtime_provider_command_from_config(
    data_root: &std::path::Path,
    provider_id: &str,
) -> anyhow::Result<Option<installer::ProviderRuntimeCommand>> {
    let cfg = installer::load_agent_server_config(data_root)
        .await
        .context("loading agent server config")?;
    installer::resolve_runtime_provider_command(&cfg, provider_id)
        .with_context(|| format!("resolving runtime command for {provider_id}"))
}

pub(super) async fn resolve_provider_login_command_from_config(
    data_root: &std::path::Path,
    provider_id: &str,
) -> anyhow::Result<Option<installer::ProviderRuntimeCommand>> {
    let cfg = installer::load_agent_server_config(data_root)
        .await
        .context("loading agent server config")?;
    installer::resolve_provider_login_command(&cfg, provider_id)
        .with_context(|| format!("resolving prepared login executable for {provider_id}"))
}
