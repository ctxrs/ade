use super::*;
use crate::api::MobileAuthContext;
use axum::Extension;
use ctx_core::provider_policy::CODEX_APP_SERVER_ARGS;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};

mod auth_url;
mod browser;
mod callback;
mod claude;
mod codex;
mod email;
mod kimi;
mod mistral;
mod notices;
mod timeouts;

pub(super) use auth_url::{
    auth_url_looks_complete, extract_auth_url, normalize_claude_login_line,
    read_trailing_claude_login_lines,
};
pub(crate) use browser::{
    get_amp_login, get_gemini_login, get_qwen_login, start_amp_login, start_gemini_login,
    start_qwen_login,
};
pub(super) use callback::{
    expected_callback_from_auth_url, is_loopback_host, validate_callback_url,
};
#[cfg(test)]
pub(super) use claude::resolve_claude_login_runtime_from_config;
pub(crate) use claude::{get_claude_login, start_claude_login};
pub(crate) use codex::{complete_codex_login, get_codex_login, start_codex_login};
pub(super) use email::{first_email_from_google_accounts, first_email_from_value};
pub(crate) use kimi::{get_kimi_login, start_kimi_login};
pub(crate) use mistral::{get_mistral_login, start_mistral_login};
pub(super) use notices::{
    auth_notice_code, is_auth_failure_notice_code, is_auth_success_notice_code,
};
pub(super) use timeouts::{
    amp_login_timeout, gemini_login_timeout, mistral_login_timeout, qwen_login_timeout,
};

const CODEX_LOGIN_RPC_TIMEOUT: Duration = Duration::from_secs(30);
const CLAUDE_LOGIN_URL_WAIT: Duration = Duration::from_secs(20);
const GEMINI_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const QWEN_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
const AMP_LOGIN_POLL_INTERVAL: Duration = Duration::from_millis(700);
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
