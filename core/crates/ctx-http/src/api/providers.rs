use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path as StdPath, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::Json;
use chrono::{DateTime, Utc};
use futures::{Stream, StreamExt};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use super::errors::ApiErrorResp;
use crate::daemon::{normalize_acp_provider_command, AppState};
use crate::execution_effective;
use crate::harness_sources;
use crate::harness_sources::{
    HarnessApiShape, HarnessEndpointUpsert, HarnessEndpointVerificationStatus, HarnessSourceKind,
};
use crate::installer;
use crate::installs::{InstallId, InstallInfo, InstallProgressEvent, InstallTarget};
use crate::logs;
use crate::provider_accounts;
use crate::provider_auth_import;
use crate::provider_probe;
use crate::provider_usage;
use crate::settings::ExecutionMode;
use ctx_core::ids::WorkspaceId;
use ctx_providers::adapters::{ProviderRestartMode, ProviderStatus};
use ctx_providers::crp::probe_crp_models;

use super::redact_json_value;

fn invalid_provider_id_error(
    provider_id: &str,
    canonical_id: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": format!(
                "provider '{}' is not supported; use '{}'",
                provider_id, canonical_id
            ),
            "code": "invalid_provider_id",
            "provider_id": provider_id,
            "canonical_id": canonical_id,
        })),
    )
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct InstallTargetQuery {
    target: Option<String>,
}

pub(super) async fn list_providers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<Vec<ProviderStatus>>, StatusCode> {
    let target = installer::parse_install_target(query.target.as_deref())
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(providers_statuses_response(&state, target).await))
}

async fn providers_statuses_response(
    state: &Arc<AppState>,
    target: InstallTarget,
) -> Vec<ProviderStatus> {
    let map = state.providers.statuses.lock().await;
    let mut out: Vec<ProviderStatus> = map.values().cloned().collect();
    drop(map);

    let managed = installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;

    let show_fake = std::env::var("CTX_SHOW_FAKE_PROVIDER").ok().as_deref() == Some("1");
    for status in out.iter_mut() {
        installer::apply_managed_install_details(status, &managed);
        if status.provider_id == "fake" {
            status.details.insert(
                "ui_hidden".into(),
                if show_fake { "false" } else { "true" }.into(),
            );
        }
        status.details.insert(
            "install_supported".into(),
            if installer::is_supported_managed_provider_for_target(
                &matrix,
                &status.provider_id,
                target,
            ) {
                "true".into()
            } else {
                "false".into()
            },
        );
        status
            .details
            .insert("install_target".into(), target.as_str().to_string());
        if let Some(bytes) =
            installer::managed_install_download_size_bytes(&matrix, &status.provider_id, target)
        {
            status
                .details
                .insert("install_download_size_bytes".into(), bytes.to_string());
        }
        if let Some(install_id) = state
            .find_running_install(&status.provider_id, Some(target))
            .await
        {
            status
                .details
                .insert("install_running".into(), "true".into());
            status
                .details
                .insert("install_id".into(), install_id.to_string());
        }
    }
    out
}

pub(super) async fn get_provider(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<ProviderStatus>, (StatusCode, Json<serde_json::Value>)> {
    if id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let map = state.providers.statuses.lock().await;
    let mut status = map.get(&id).cloned().ok_or((
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": format!("provider not found: {id}")
        })),
    ))?;
    drop(map);
    let target = installer::parse_install_target(query.target.as_deref()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    let managed = installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    installer::apply_managed_install_details(&mut status, &managed);
    status.details.insert(
        "install_supported".into(),
        if installer::is_supported_managed_provider_for_target(&matrix, &status.provider_id, target)
        {
            "true".into()
        } else {
            "false".into()
        },
    );
    status
        .details
        .insert("install_target".into(), target.as_str().to_string());
    if let Some(bytes) =
        installer::managed_install_download_size_bytes(&matrix, &status.provider_id, target)
    {
        status
            .details
            .insert("install_download_size_bytes".into(), bytes.to_string());
    }
    if let Some(install_id) = state.find_running_install(&id, Some(target)).await {
        status
            .details
            .insert("install_running".into(), "true".into());
        status
            .details
            .insert("install_id".into(), install_id.to_string());
    }
    Ok(Json(status))
}

#[derive(Debug, Deserialize)]
pub(super) struct ProviderUsageQuery {
    refresh: Option<bool>,
}

pub(super) async fn get_provider_usage(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ProviderUsageQuery>,
) -> Result<Json<provider_usage::ProviderUsageSnapshot>, (StatusCode, Json<serde_json::Value>)> {
    if id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let refresh = query.refresh.unwrap_or(false);
    let snapshot = if !refresh {
        let cache = state.providers.usage_cache.lock().await;
        cache.get(&id).cloned()
    } else {
        None
    };
    let snapshot = match snapshot {
        Some(snapshot) => snapshot,
        None => {
            let env = if id == "codex" {
                provider_accounts::codex_env_for_active_account(&state.core.data_root)
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": e.to_string()
                            })),
                        )
                    })?
            } else {
                HashMap::new()
            };
            provider_usage::refresh_provider_usage_for(&state, &id, env)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": e.to_string()
                        })),
                    )
                })?
        }
    };
    Ok(Json(snapshot))
}

#[derive(Debug, Serialize)]
pub(super) struct CodexAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CodexAccountEntry>,
    logins: Vec<provider_accounts::CodexLoginStatus>,
}

#[derive(Debug, Serialize)]
pub(super) struct ClaudeAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::ClaudeAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct GeminiAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::GeminiAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct QwenAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::QwenAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct KimiAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::KimiAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct MistralAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::MistralAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct CopilotAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CopilotAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct CursorAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CursorAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct ProvidersBootstrapResponse {
    providers: Vec<ProviderStatus>,
    provider_options: HashMap<String, serde_json::Value>,
    provider_harness_config: HashMap<String, harness_sources::HarnessProviderSourceConfig>,
    codex_accounts: CodexAccountsResponse,
    claude_accounts: ClaudeAccountsResponse,
    gemini_accounts: GeminiAccountsResponse,
    qwen_accounts: QwenAccountsResponse,
    kimi_accounts: KimiAccountsResponse,
    mistral_accounts: MistralAccountsResponse,
    copilot_accounts: CopilotAccountsResponse,
    cursor_accounts: CursorAccountsResponse,
    amp_accounts: AmpAccountsResponse,
}

#[derive(Debug, Serialize)]
pub(super) struct AmpAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::AmpAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct CodexAccountUsageEntry {
    account_id: Option<String>,
    label: String,
    email: Option<String>,
    plan_type: Option<String>,
    last_used_at: Option<DateTime<Utc>>,
    usage: provider_usage::ProviderUsageSnapshot,
}

#[derive(Debug, Serialize)]
pub(super) struct CodexAccountsUsageResponse {
    entries: Vec<CodexAccountUsageEntry>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CodexLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct CodexLoginStartResp {
    account_id: String,
    auth_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_callback_url: Option<String>,
    completion_token: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct CodexActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaudeAccountUpsertReq {
    label: Option<String>,
    #[serde(alias = "auth_token")]
    setup_token: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaudeActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaudeLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ClaudeLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaudeLoginCompleteReq {
    callback_code: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ClaudeLoginCompleteResp {
    accepted: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct GeminiAccountUpsertReq {
    label: Option<String>,
    oauth_creds_json: String,
    #[serde(default)]
    google_accounts_json: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GeminiLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct GeminiLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct QwenLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct QwenLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AmpLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct AmpLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MistralLoginStartReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct MistralLoginStartResp {
    login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GeminiActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct QwenActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KimiAccountUpsertReq {
    label: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    credentials_json: String,
    #[serde(default)]
    config_toml: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KimiActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MistralActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CopilotAccountUpsertReq {
    label: Option<String>,
    token: String,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CopilotActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CursorAccountUpsertReq {
    label: Option<String>,
    token: String,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CursorActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AmpActiveAccountReq {
    account_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ProviderAuthImportCandidatesResponse {
    candidates: Vec<provider_auth_import::ProviderAuthImportCandidate>,
}

#[derive(Debug, Serialize)]
pub(super) struct ProviderAuthImportProfilesResponse {
    profiles: Vec<provider_auth_import::ProviderImportedAuthProfile>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ProviderAuthImportReq {
    candidate_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ProviderAuthImportResponse {
    results: Vec<provider_auth_import::ProviderAuthImportResult>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CodexHostImportReq {
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CodexLoginCompleteReq {
    callback_url: String,
    completion_token: String,
}

#[derive(Debug, Serialize)]
pub(super) struct CodexLoginCompleteResp {
    accepted: bool,
    status_code: u16,
}

pub(super) struct CodexLoginProcess {
    login_id: String,
    auth_url: String,
    account_dir: PathBuf,
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    reader: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
}

pub(super) struct CodexLoginCompletion {
    success: bool,
    error: Option<String>,
}

pub(super) struct ClaudeLoginProcess {
    line_rx: mpsc::UnboundedReceiver<String>,
    input_rx: mpsc::UnboundedReceiver<String>,
    buffered_lines: Vec<String>,
    auth_url: Option<String>,
    exit_rx: oneshot::Receiver<anyhow::Result<portable_pty::ExitStatus>>,
    killer: Arc<StdMutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    writer: Arc<StdMutex<Box<dyn Write + Send>>>,
}

struct ClaudeLoginSpawn {
    line_rx: mpsc::UnboundedReceiver<String>,
    exit_rx: oneshot::Receiver<anyhow::Result<portable_pty::ExitStatus>>,
    killer: Arc<StdMutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    writer: Arc<StdMutex<Box<dyn Write + Send>>>,
}

const CODEX_LOGIN_RPC_TIMEOUT: Duration = Duration::from_secs(30);
const CLAUDE_LOGIN_URL_WAIT: Duration = Duration::from_secs(4);
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
const CLAUDE_LOGIN_NO_AUTH_URL_TIMEOUT: Duration = Duration::from_secs(8);
const CLAUDE_LOGIN_URL_SETTLE_WAIT: Duration = Duration::from_millis(500);
const CLAUDE_LOGIN_COMPLETION_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const CLAUDE_LOGIN_EXIT_GRACE_WAIT: Duration = Duration::from_millis(400);

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

fn expected_callback_from_auth_url(auth_url: &str) -> Option<String> {
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

fn validate_callback_url(
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

fn extract_auth_url_from_value(value: &serde_json::Value) -> Option<String> {
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
    matches!(code, "auth_failed" | "auth_error")
}

async fn codex_accounts_response(state: &Arc<AppState>) -> CodexAccountsResponse {
    let registry = provider_accounts::load_codex_registry(&state.core.data_root).await;
    let logins = {
        let map = state.providers.codex_login_sessions.lock().await;
        map.values().cloned().collect::<Vec<_>>()
    };
    CodexAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    }
}

async fn claude_accounts_response(state: &Arc<AppState>) -> ClaudeAccountsResponse {
    let registry = provider_accounts::load_claude_registry(&state.core.data_root).await;
    ClaudeAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

async fn gemini_accounts_response(state: &Arc<AppState>) -> GeminiAccountsResponse {
    let registry = provider_accounts::load_gemini_registry(&state.core.data_root).await;
    GeminiAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

async fn qwen_accounts_response(state: &Arc<AppState>) -> QwenAccountsResponse {
    let registry = provider_accounts::load_qwen_registry(&state.core.data_root).await;
    QwenAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

async fn kimi_accounts_response(state: &Arc<AppState>) -> KimiAccountsResponse {
    let registry = provider_accounts::load_kimi_registry(&state.core.data_root).await;
    KimiAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

async fn mistral_accounts_response(state: &Arc<AppState>) -> MistralAccountsResponse {
    let registry = provider_accounts::load_mistral_registry(&state.core.data_root).await;
    MistralAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

async fn copilot_accounts_response(state: &Arc<AppState>) -> CopilotAccountsResponse {
    let registry = provider_accounts::load_copilot_registry(&state.core.data_root).await;
    CopilotAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

async fn cursor_accounts_response(state: &Arc<AppState>) -> CursorAccountsResponse {
    let registry = provider_accounts::load_cursor_registry(&state.core.data_root).await;
    CursorAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    }
}

async fn amp_accounts_response(state: &Arc<AppState>) -> anyhow::Result<AmpAccountsResponse> {
    let registry =
        provider_accounts::ensure_amp_registry_from_runtime_auth(&state.core.data_root).await?;
    Ok(AmpAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
    })
}

async fn restart_provider_for_auth_change(state: &Arc<AppState>, provider_id: &str, reason: &str) {
    let adapters = {
        let map = state.providers.adapters.lock().await;
        [provider_id]
            .iter()
            .filter_map(|id| {
                map.get(*id)
                    .map(|adapter| (id.to_string(), Arc::clone(adapter)))
            })
            .collect::<Vec<_>>()
    };
    for (id, adapter) in adapters {
        if let Err(err) = adapter.restart(reason, ProviderRestartMode::Drain).await {
            tracing::warn!("failed to drain-restart {id} after auth change: {err}");
        }
    }
}

async fn restart_codex_providers_for_auth_change(state: &Arc<AppState>, reason: &str) {
    restart_provider_for_auth_change(state, "codex", reason).await;
}

async fn restart_claude_providers_for_auth_change(state: &Arc<AppState>, reason: &str) {
    restart_provider_for_auth_change(state, "claude-crp", reason).await;
}

async fn restart_gemini_providers_for_auth_change(state: &Arc<AppState>, reason: &str) {
    restart_provider_for_auth_change(state, "gemini", reason).await;
}

async fn restart_qwen_providers_for_auth_change(state: &Arc<AppState>, reason: &str) {
    restart_provider_for_auth_change(state, "qwen", reason).await;
}

async fn restart_amp_providers_for_auth_change(state: &Arc<AppState>, reason: &str) {
    restart_provider_for_auth_change(state, "amp", reason).await;
}

async fn restart_mistral_providers_for_auth_change(state: &Arc<AppState>, reason: &str) {
    restart_provider_for_auth_change(state, "mistral", reason).await;
}

async fn restart_kimi_providers_for_auth_change(state: &Arc<AppState>, reason: &str) {
    restart_provider_for_auth_change(state, "kimi", reason).await;
}

async fn restart_copilot_providers_for_auth_change(state: &Arc<AppState>, reason: &str) {
    restart_provider_for_auth_change(state, "copilot", reason).await;
}

async fn restart_cursor_providers_for_auth_change(state: &Arc<AppState>, reason: &str) {
    restart_provider_for_auth_change(state, "cursor", reason).await;
}

fn import_result_requires_provider_restart(
    result: &provider_auth_import::ProviderAuthImportResult,
) -> bool {
    // `already_imported` can still mutate active account selection (dedupe/upsert paths),
    // so treat it as auth-affecting to avoid stale runtime credentials.
    matches!(
        result.status.as_str(),
        "imported" | "updated" | "already_imported"
    )
}

pub(super) async fn list_codex_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(codex_accounts_response(&state).await))
}

pub(super) async fn probe_host_codex_import(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<provider_accounts::CodexHostImportProbe>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(
        provider_accounts::probe_host_codex_auth_candidate().await,
    ))
}

pub(super) async fn import_host_codex_auth(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodexHostImportReq>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::import_host_codex_auth_to_secret_store(&state.core.data_root, req.label)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_codex_providers_for_auth_change(&state, "codex auth updated").await;
    Ok(Json(codex_accounts_response(&state).await))
}

pub(super) async fn complete_codex_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CodexLoginCompleteReq>,
) -> Result<Json<CodexLoginCompleteResp>, (StatusCode, Json<ApiErrorResp>)> {
    let expected_callback = {
        let map = state.providers.codex_login_sessions.lock().await;
        let Some(status) = map.get(&id) else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "login not found".to_string(),
                }),
            ));
        };
        if status.status != "pending" {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "login is not pending".to_string(),
                }),
            ));
        }
        if status.completion_token.as_deref() != Some(req.completion_token.as_str()) {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResp {
                    error: "invalid completion token".to_string(),
                }),
            ));
        }
        status.expected_callback_url.clone()
    };

    validate_callback_url(&req.callback_url, expected_callback.as_deref()).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: err.to_string(),
            }),
        )
    })?;

    let response = reqwest::Client::new()
        .get(&req.callback_url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResp {
                    error: format!("failed to replay callback: {err}"),
                }),
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(ApiErrorResp {
                error: format!("callback replay returned {status}"),
            }),
        ));
    }
    let status_code = status.as_u16();

    {
        let mut map = state.providers.codex_login_sessions.lock().await;
        if let Some(status) = map.get_mut(&id) {
            status.completion_token = None;
        }
    }

    Ok(Json(CodexLoginCompleteResp {
        accepted: true,
        status_code,
    }))
}

pub(super) async fn get_codex_accounts_usage(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProviderUsageQuery>,
) -> Result<Json<CodexAccountsUsageResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let refresh = query.refresh.unwrap_or(false);
    let registry = provider_accounts::load_codex_registry(&state.core.data_root).await;
    let active_id = registry.active_account_id.clone();
    let cached_active = if !refresh {
        let cache = state.providers.usage_cache.lock().await;
        cache.get("codex").cloned()
    } else {
        None
    };

    let to_err = |e: anyhow::Error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    };

    let mut entries = Vec::new();

    for account in registry.accounts {
        let _ = provider_accounts::hydrate_codex_account_home_from_secret(
            &state.core.data_root,
            &account.id,
        )
        .await;
        let env = provider_accounts::codex_env_for_account(&state.core.data_root, &account.id);
        let usage = if active_id.as_deref() == Some(&account.id) {
            if let Some(snapshot) = cached_active.clone() {
                snapshot
            } else {
                provider_usage::fetch_codex_usage_snapshot(env)
                    .await
                    .map_err(to_err)?
            }
        } else {
            provider_usage::fetch_codex_usage_snapshot(env)
                .await
                .map_err(to_err)?
        };
        entries.push(CodexAccountUsageEntry {
            account_id: Some(account.id),
            label: account.label,
            email: account.email,
            plan_type: account.plan_type,
            last_used_at: account.last_used_at,
            usage,
        });
    }

    Ok(Json(CodexAccountsUsageResponse { entries }))
}

pub(super) async fn start_codex_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodexLoginStartReq>,
) -> Result<Json<CodexLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    let account_id = uuid::Uuid::new_v4().to_string();
    let label = provider_accounts::normalize_label(req.label, &account_id);
    let account_dir =
        provider_accounts::ensure_codex_account_dir(&state.core.data_root, &account_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: e.to_string(),
                    }),
                )
            })?;
    let login = match start_codex_login_process(&account_dir).await {
        Ok(login) => login,
        Err(e) => {
            let _ = tokio::fs::remove_dir_all(&account_dir).await;
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            ));
        }
    };
    let expected_callback_url = expected_callback_from_auth_url(&login.auth_url);
    let completion_token = uuid::Uuid::new_v4().to_string();
    let auth_url = login.auth_url.clone();
    let status = provider_accounts::CodexLoginStatus {
        account_id: account_id.clone(),
        auth_url: auth_url.clone(),
        expected_callback_url: expected_callback_url.clone(),
        completion_token: Some(completion_token.clone()),
        status: "pending".to_string(),
        error: None,
    };
    {
        let mut map = state.providers.codex_login_sessions.lock().await;
        map.insert(account_id.clone(), status);
    }
    let state_clone = Arc::clone(&state);
    let account_id_for_task = account_id.clone();
    tokio::spawn(async move {
        monitor_codex_login(state_clone, account_id_for_task, label, login).await;
    });

    Ok(Json(CodexLoginStartResp {
        account_id,
        auth_url,
        expected_callback_url,
        completion_token,
    }))
}

pub(super) async fn get_codex_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::CodexLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let map = state.providers.codex_login_sessions.lock().await;
    let status = map.get(&id).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}

pub(super) async fn set_codex_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CodexActiveAccountReq>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_codex_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    let registry =
        provider_accounts::set_active_codex_account(&state.core.data_root, req.account_id)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                let status = if msg.contains("api_shape=openai_responses")
                    || msg.contains("auth_type=bearer")
                    || msg.contains("unknown account")
                {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                (status, Json(ApiErrorResp { error: msg }))
            })?;
    restart_codex_providers_for_auth_change(&state, "codex auth updated").await;
    let logins = {
        let map = state.providers.codex_login_sessions.lock().await;
        map.values().cloned().collect::<Vec<_>>()
    };
    Ok(Json(CodexAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    }))
}

pub(super) async fn delete_codex_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let registry = provider_accounts::remove_codex_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_codex_providers_for_auth_change(&state, "codex auth updated").await;
    let logins = {
        let mut map = state.providers.codex_login_sessions.lock().await;
        map.remove(&id);
        map.values().cloned().collect::<Vec<_>>()
    };
    Ok(Json(CodexAccountsResponse {
        active_account_id: registry.active_account_id,
        accounts: registry.accounts,
        logins,
    }))
}

pub(super) async fn list_claude_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(claude_accounts_response(&state).await))
}

pub(super) async fn start_claude_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeLoginStartReq>,
) -> Result<Json<ClaudeLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    let login_id = uuid::Uuid::new_v4().to_string();
    let label = req.label;
    let mut login = start_claude_login_process(&state).await.map_err(|e| {
        let msg = e.to_string();
        let status = if msg.contains("runtime_command_") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, Json(ApiErrorResp { error: msg }))
    })?;
    let (input_tx, input_rx) = mpsc::unbounded_channel::<String>();
    {
        let mut map = state.providers.claude_login_inputs.lock().await;
        map.insert(login_id.clone(), input_tx);
    }
    login.input_rx = input_rx;
    let auth_url = login.auth_url.clone();
    let status = provider_accounts::ClaudeLoginStatus {
        login_id: login_id.clone(),
        auth_url: auth_url.clone(),
        status: "pending".to_string(),
        account_id: None,
        error: None,
    };
    {
        let mut map = state.providers.claude_login_sessions.lock().await;
        map.insert(login_id.clone(), status);
    }
    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_id.clone();
    tokio::spawn(async move {
        monitor_claude_login(state_clone, login_id_for_task, label, login).await;
    });

    Ok(Json(ClaudeLoginStartResp { login_id, auth_url }))
}

pub(super) async fn complete_claude_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<ClaudeLoginCompleteReq>,
) -> Result<Json<ClaudeLoginCompleteResp>, (StatusCode, Json<ApiErrorResp>)> {
    let callback_code = req.callback_code.trim();
    if callback_code.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "callback_code is required".to_string(),
            }),
        ));
    }
    {
        let sessions = state.providers.claude_login_sessions.lock().await;
        let Some(status) = sessions.get(&id) else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "login not found".to_string(),
                }),
            ));
        };
        if status.status != "pending" {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "login is no longer pending".to_string(),
                }),
            ));
        }
    }
    let tx = {
        let map = state.providers.claude_login_inputs.lock().await;
        map.get(&id).cloned()
    }
    .ok_or_else(|| {
        (
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "login session is not accepting callback input".to_string(),
            }),
        )
    })?;
    tx.send(callback_code.to_string()).map_err(|_| {
        (
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "login session is no longer accepting callback input".to_string(),
            }),
        )
    })?;
    Ok(Json(ClaudeLoginCompleteResp { accepted: true }))
}

pub(super) async fn get_claude_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::ClaudeLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let map = state.providers.claude_login_sessions.lock().await;
    let status = map.get(&id).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}

pub(super) async fn upsert_claude_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeAccountUpsertReq>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_claude_account(&state.core.data_root, req.label, req.setup_token)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_claude_providers_for_auth_change(&state, "claude auth updated").await;
    Ok(Json(claude_accounts_response(&state).await))
}

pub(super) async fn set_claude_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeActiveAccountReq>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_claude_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_claude_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_claude_providers_for_auth_change(&state, "claude auth updated").await;
    Ok(Json(claude_accounts_response(&state).await))
}

pub(super) async fn delete_claude_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_claude_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_claude_providers_for_auth_change(&state, "claude auth updated").await;
    Ok(Json(claude_accounts_response(&state).await))
}

fn gemini_login_home(data_root: &StdPath, login_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("gemini")
        .join("login-sessions")
        .join(login_id)
}

async fn monitor_gemini_login(state: Arc<AppState>, login_id: String, label: Option<String>) {
    let adapter = {
        let map = state.providers.adapters.lock().await;
        map.get("gemini").cloned()
    };
    let Some(adapter) = adapter else {
        let mut map = state.providers.gemini_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some("provider adapter not available".to_string());
        }
        return;
    };

    let login_home = gemini_login_home(&state.core.data_root, &login_id);
    let workdir = login_home.join("workspace");
    if let Err(err) = tokio::fs::create_dir_all(&workdir).await {
        let mut map = state.providers.gemini_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(format!("failed to prepare login workspace: {err}"));
        }
        return;
    }

    let mut provider_env = HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    if let Some(token) = state.core.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
    provider_env.insert(
        "GEMINI_CLI_HOME".to_string(),
        login_home.to_string_lossy().to_string(),
    );
    provider_env.insert(
        provider_accounts::GEMINI_FORCE_FILE_STORAGE_ENV.to_string(),
        "true".to_string(),
    );
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.core.data_root.to_string_lossy().to_string(),
    );

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let auth_result = adapter
        .authenticate_session(
            format!("gemini-login-{}", login_id),
            workdir,
            provider_env,
            Some(provider_accounts::GEMINI_CREDENTIAL_KIND_OAUTH_PERSONAL.to_string()),
            event_tx,
        )
        .await;
    if let Err(err) = auth_result {
        let mut map = state.providers.gemini_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(logs::redact_sensitive(&err.to_string()));
        }
        let _ = tokio::fs::remove_dir_all(&login_home).await;
        return;
    }

    let oauth_path = login_home.join(".gemini").join("oauth_creds.json");
    let google_accounts_path = login_home.join(".gemini").join("google_accounts.json");
    let started_at = Instant::now();
    let timeout = gemini_login_timeout();
    let mut observed_auth_url = false;

    loop {
        let mut channel_disconnected = false;
        loop {
            match event_rx.try_recv() {
                Ok(event) => {
                    if let Some(auth_url) = extract_auth_url_from_value(&event.payload_json) {
                        observed_auth_url = true;
                        let mut map = state.providers.gemini_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.auth_url = Some(auth_url);
                        }
                    }
                    if matches!(event.event_type, ctx_core::models::SessionEventType::Error) {
                        let message = event
                            .payload_json
                            .get("message")
                            .and_then(serde_json::Value::as_str)
                            .map(logs::redact_sensitive)
                            .unwrap_or_else(|| "gemini authenticate reported an error".to_string());
                        let mut map = state.providers.gemini_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.status = "failed".to_string();
                            entry.error = Some(message);
                        }
                        let _ = tokio::fs::remove_dir_all(&login_home).await;
                        return;
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    channel_disconnected = true;
                    break;
                }
            }
        }

        let oauth_raw = tokio::fs::read_to_string(&oauth_path)
            .await
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(oauth_raw) = oauth_raw {
            let oauth_value = serde_json::from_str::<serde_json::Value>(&oauth_raw);
            let oauth_valid = oauth_value
                .as_ref()
                .ok()
                .is_some_and(serde_json::Value::is_object);
            if !oauth_valid {
                let mut map = state.providers.gemini_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    entry.error =
                        Some("captured oauth_creds.json is not a valid JSON object".to_string());
                }
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }

            let google_accounts_raw = tokio::fs::read_to_string(&google_accounts_path)
                .await
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let google_accounts_value = google_accounts_raw
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
            let email = google_accounts_value
                .as_ref()
                .and_then(first_email_from_google_accounts);
            let added = provider_accounts::add_gemini_account(
                &state.core.data_root,
                label.clone(),
                oauth_raw,
                google_accounts_raw,
                email,
            )
            .await;
            match added {
                Ok(registry) => {
                    let mut map = state.providers.gemini_login_sessions.lock().await;
                    if let Some(entry) = map.get_mut(&login_id) {
                        entry.status = "success".to_string();
                        entry.account_id = registry.active_account_id.clone();
                        entry.error = None;
                    }
                    restart_gemini_providers_for_auth_change(&state, "gemini auth updated").await;
                }
                Err(err) => {
                    let mut map = state.providers.gemini_login_sessions.lock().await;
                    if let Some(entry) = map.get_mut(&login_id) {
                        entry.status = "failed".to_string();
                        entry.error = Some(logs::redact_sensitive(&err.to_string()));
                    }
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        if channel_disconnected && !observed_auth_url {
            let mut map = state.providers.gemini_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "failed".to_string();
                if entry.error.is_none() {
                    entry.error = Some(
                        "Gemini sign-in did not emit an OAuth URL; the runtime may require API-key auth in this environment."
                            .to_string(),
                    );
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        if started_at.elapsed() >= timeout {
            let mut map = state.providers.gemini_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "timeout".to_string();
                if entry.error.is_none() {
                    entry.error = Some("timed out waiting for Gemini OAuth completion".to_string());
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        tokio::time::sleep(GEMINI_LOGIN_POLL_INTERVAL).await;
    }
}

pub(super) async fn start_gemini_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GeminiLoginStartReq>,
) -> Result<Json<GeminiLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    let login_id = uuid::Uuid::new_v4().to_string();
    {
        let mut map = state.providers.gemini_login_sessions.lock().await;
        map.insert(
            login_id.clone(),
            provider_accounts::GeminiLoginStatus {
                login_id: login_id.clone(),
                auth_url: None,
                status: "pending".to_string(),
                account_id: None,
                error: None,
            },
        );
    }

    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_id.clone();
    tokio::spawn(async move {
        monitor_gemini_login(state_clone, login_id_for_task, req.label).await;
    });

    Ok(Json(GeminiLoginStartResp {
        login_id,
        auth_url: None,
    }))
}

pub(super) async fn get_gemini_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::GeminiLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let map = state.providers.gemini_login_sessions.lock().await;
    let status = map.get(&id).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}

fn qwen_login_home(data_root: &StdPath, login_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("qwen")
        .join("login-sessions")
        .join(login_id)
}

async fn monitor_qwen_login(state: Arc<AppState>, login_id: String, label: Option<String>) {
    let adapter = {
        let map = state.providers.adapters.lock().await;
        map.get("qwen").cloned()
    };
    let Some(adapter) = adapter else {
        let mut map = state.providers.qwen_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some("provider adapter not available".to_string());
        }
        return;
    };

    let login_home = qwen_login_home(&state.core.data_root, &login_id);
    let workdir = login_home.join("workspace");
    if let Err(err) = tokio::fs::create_dir_all(&workdir).await {
        let mut map = state.providers.qwen_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(format!("failed to prepare login workspace: {err}"));
        }
        return;
    }
    let _ = tokio::fs::create_dir_all(login_home.join(".config")).await;
    let _ = tokio::fs::create_dir_all(login_home.join(".cache")).await;

    let mut provider_env = HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    if let Some(token) = state.core.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.core.data_root.to_string_lossy().to_string(),
    );
    provider_env.insert("HOME".to_string(), login_home.to_string_lossy().to_string());
    provider_env.insert(
        "XDG_CONFIG_HOME".to_string(),
        login_home.join(".config").to_string_lossy().to_string(),
    );
    provider_env.insert(
        "XDG_CACHE_HOME".to_string(),
        login_home.join(".cache").to_string_lossy().to_string(),
    );

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let auth_result = adapter
        .authenticate_session(
            format!("qwen-login-{login_id}"),
            workdir,
            provider_env,
            Some(QWEN_OAUTH_AUTH_METHOD_ID.to_string()),
            event_tx,
        )
        .await;
    if let Err(err) = auth_result {
        let mut map = state.providers.qwen_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(logs::redact_sensitive(&err.to_string()));
        }
        let _ = tokio::fs::remove_dir_all(&login_home).await;
        return;
    }

    let oauth_path = login_home.join(provider_accounts::QWEN_OAUTH_CREDS_RELATIVE_PATH);
    let started_at = Instant::now();
    let timeout = qwen_login_timeout();
    let mut observed_auth_url = false;
    let mut observed_email = None::<String>;

    loop {
        let mut channel_disconnected = false;
        loop {
            match event_rx.try_recv() {
                Ok(event) => {
                    if let Some(auth_url) = extract_auth_url_from_value(&event.payload_json) {
                        observed_auth_url = true;
                        let mut map = state.providers.qwen_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.auth_url = Some(auth_url);
                        }
                    }
                    if observed_email.is_none() {
                        observed_email = first_email_from_value(&event.payload_json);
                    }
                    if matches!(event.event_type, ctx_core::models::SessionEventType::Error) {
                        let message = event
                            .payload_json
                            .get("message")
                            .and_then(serde_json::Value::as_str)
                            .map(logs::redact_sensitive)
                            .unwrap_or_else(|| "qwen authenticate reported an error".to_string());
                        let mut map = state.providers.qwen_login_sessions.lock().await;
                        if let Some(entry) = map.get_mut(&login_id) {
                            entry.status = "failed".to_string();
                            entry.error = Some(message);
                        }
                        let _ = tokio::fs::remove_dir_all(&login_home).await;
                        return;
                    }
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    channel_disconnected = true;
                    break;
                }
            }
        }

        let oauth_raw = tokio::fs::read_to_string(&oauth_path)
            .await
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(oauth_raw) = oauth_raw {
            let oauth_value = serde_json::from_str::<serde_json::Value>(&oauth_raw);
            let oauth_valid = oauth_value
                .as_ref()
                .ok()
                .is_some_and(serde_json::Value::is_object);
            if !oauth_valid {
                let mut map = state.providers.qwen_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    entry.error =
                        Some("captured oauth_creds.json is not a valid JSON object".to_string());
                }
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }

            let added = provider_accounts::add_qwen_account(
                &state.core.data_root,
                label.clone(),
                oauth_raw,
                observed_email.clone(),
            )
            .await;
            match added {
                Ok(registry) => {
                    let mut map = state.providers.qwen_login_sessions.lock().await;
                    if let Some(entry) = map.get_mut(&login_id) {
                        entry.status = "success".to_string();
                        entry.account_id = registry.active_account_id.clone();
                        entry.error = None;
                    }
                    restart_qwen_providers_for_auth_change(&state, "qwen auth updated").await;
                }
                Err(err) => {
                    let mut map = state.providers.qwen_login_sessions.lock().await;
                    if let Some(entry) = map.get_mut(&login_id) {
                        entry.status = "failed".to_string();
                        entry.error = Some(logs::redact_sensitive(&err.to_string()));
                    }
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        if channel_disconnected && !observed_auth_url {
            let mut map = state.providers.qwen_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "failed".to_string();
                if entry.error.is_none() {
                    entry.error = Some(
                        "Qwen sign-in did not emit an OAuth URL in this environment.".to_string(),
                    );
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        if started_at.elapsed() >= timeout {
            let mut map = state.providers.qwen_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "timeout".to_string();
                if entry.error.is_none() {
                    entry.error = Some("timed out waiting for Qwen OAuth completion".to_string());
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        tokio::time::sleep(QWEN_LOGIN_POLL_INTERVAL).await;
    }
}

pub(super) async fn start_qwen_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QwenLoginStartReq>,
) -> Result<Json<QwenLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    let login_id = uuid::Uuid::new_v4().to_string();
    {
        let mut map = state.providers.qwen_login_sessions.lock().await;
        map.insert(
            login_id.clone(),
            provider_accounts::QwenLoginStatus {
                login_id: login_id.clone(),
                auth_url: None,
                status: "pending".to_string(),
                account_id: None,
                error: None,
            },
        );
    }

    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_id.clone();
    tokio::spawn(async move {
        monitor_qwen_login(state_clone, login_id_for_task, req.label).await;
    });

    Ok(Json(QwenLoginStartResp {
        login_id,
        auth_url: None,
    }))
}

pub(super) async fn get_qwen_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::QwenLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let map = state.providers.qwen_login_sessions.lock().await;
    let status = map.get(&id).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}

fn amp_login_home(data_root: &StdPath, login_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("amp")
        .join("login-sessions")
        .join(login_id)
}

async fn monitor_amp_login(state: Arc<AppState>, login_id: String, label: Option<String>) {
    let adapter = {
        let map = state.providers.adapters.lock().await;
        map.get("amp").cloned()
    };
    let Some(adapter) = adapter else {
        let mut map = state.providers.amp_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some("provider adapter not available".to_string());
        }
        return;
    };

    let login_home = amp_login_home(&state.core.data_root, &login_id);
    let workdir = login_home.join("workspace");
    if let Err(err) = tokio::fs::create_dir_all(&workdir).await {
        let mut map = state.providers.amp_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(format!("failed to prepare login workspace: {err}"));
        }
        return;
    }
    let amp_home = match provider_accounts::ensure_amp_runtime_home(&state.core.data_root).await {
        Ok(home) => home,
        Err(err) => {
            let mut map = state.providers.amp_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "failed".to_string();
                entry.error = Some(format!("failed to prepare amp runtime home: {err}"));
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }
    };

    let mut provider_env = HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    if let Some(token) = state.core.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.core.data_root.to_string_lossy().to_string(),
    );
    provider_env.insert("HOME".to_string(), amp_home.to_string_lossy().to_string());
    provider_env.insert(
        "XDG_CONFIG_HOME".to_string(),
        amp_home.join(".config").to_string_lossy().to_string(),
    );
    provider_env.insert(
        "XDG_CACHE_HOME".to_string(),
        amp_home.join(".cache").to_string_lossy().to_string(),
    );

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let auth_result = adapter
        .authenticate_session(
            format!("amp-login-{login_id}"),
            workdir,
            provider_env,
            Some(AMP_BROWSER_AUTH_METHOD_ID.to_string()),
            event_tx,
        )
        .await;
    if let Err(err) = auth_result {
        let mut map = state.providers.amp_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(logs::redact_sensitive(&err.to_string()));
        }
        let _ = tokio::fs::remove_dir_all(&login_home).await;
        return;
    }

    let started_at = Instant::now();
    let timeout = amp_login_timeout();
    let mut observed_auth_url = false;
    let mut observed_email = None::<String>;

    loop {
        if started_at.elapsed() >= timeout {
            let mut map = state.providers.amp_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "timeout".to_string();
                if entry.error.is_none() {
                    entry.error = Some("timed out waiting for Amp OAuth completion".to_string());
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        let event = match tokio::time::timeout(AMP_LOGIN_POLL_INTERVAL, event_rx.recv()).await {
            Ok(Some(event)) => event,
            Ok(None) => {
                let mut map = state.providers.amp_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    if entry.error.is_none() {
                        entry.error = Some(if observed_auth_url {
                            "Amp sign-in session ended before completion.".to_string()
                        } else {
                            "Amp sign-in did not emit an OAuth URL in this environment.".to_string()
                        });
                    }
                }
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }
            Err(_) => continue,
        };

        if let Some(auth_url) = extract_auth_url_from_value(&event.payload_json) {
            observed_auth_url = true;
            let mut map = state.providers.amp_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.auth_url = Some(auth_url);
            }
        }
        if observed_email.is_none() {
            observed_email = first_email_from_value(&event.payload_json);
        }

        if matches!(event.event_type, ctx_core::models::SessionEventType::Error) {
            let message = event
                .payload_json
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(logs::redact_sensitive)
                .unwrap_or_else(|| "amp authenticate reported an error".to_string());
            let mut map = state.providers.amp_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "failed".to_string();
                entry.error = Some(message);
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        if matches!(event.event_type, ctx_core::models::SessionEventType::Notice) {
            let code = event
                .payload_json
                .get("code")
                .or_else(|| event.payload_json.get("kind"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if matches!(
                code,
                "auth_complete" | "auth_completed" | "auth_success" | "authenticated"
            ) {
                if let Err(err) = provider_accounts::upsert_amp_account(
                    &state.core.data_root,
                    label.clone(),
                    observed_email.clone(),
                )
                .await
                {
                    let mut map = state.providers.amp_login_sessions.lock().await;
                    if let Some(entry) = map.get_mut(&login_id) {
                        entry.status = "failed".to_string();
                        entry.error = Some(logs::redact_sensitive(&err.to_string()));
                    }
                    let _ = tokio::fs::remove_dir_all(&login_home).await;
                    return;
                }
                let mut map = state.providers.amp_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "success".to_string();
                    entry.auth_url = None;
                    entry.error = None;
                }
                restart_amp_providers_for_auth_change(&state, "amp auth updated").await;
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }
            if matches!(code, "auth_failed" | "auth_error") {
                let message = event
                    .payload_json
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .map(logs::redact_sensitive)
                    .unwrap_or_else(|| "Amp sign-in failed. Retry.".to_string());
                let mut map = state.providers.amp_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    entry.error = Some(message);
                }
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }
        }
    }
}

pub(super) async fn start_amp_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AmpLoginStartReq>,
) -> Result<Json<AmpLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    let label = req.label;
    let login_id = uuid::Uuid::new_v4().to_string();
    {
        let mut map = state.providers.amp_login_sessions.lock().await;
        map.insert(
            login_id.clone(),
            provider_accounts::AmpLoginStatus {
                login_id: login_id.clone(),
                auth_url: None,
                status: "pending".to_string(),
                error: None,
            },
        );
    }

    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_id.clone();
    tokio::spawn(async move {
        monitor_amp_login(state_clone, login_id_for_task, label).await;
    });

    Ok(Json(AmpLoginStartResp {
        login_id,
        auth_url: None,
    }))
}

pub(super) async fn get_amp_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::AmpLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let map = state.providers.amp_login_sessions.lock().await;
    let status = map.get(&id).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}

fn mistral_login_home(data_root: &StdPath, login_id: &str) -> PathBuf {
    data_root
        .join("providers")
        .join("mistral")
        .join("login-sessions")
        .join(login_id)
}

async fn monitor_mistral_login(state: Arc<AppState>, login_id: String, label: Option<String>) {
    let adapter = {
        let map = state.providers.adapters.lock().await;
        map.get("mistral").cloned()
    };
    let Some(adapter) = adapter else {
        let mut map = state.providers.mistral_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some("provider adapter not available".to_string());
        }
        return;
    };

    let login_home = mistral_login_home(&state.core.data_root, &login_id);
    let workdir = login_home.join("workspace");
    if let Err(err) = tokio::fs::create_dir_all(&workdir).await {
        let mut map = state.providers.mistral_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(format!("failed to prepare login workspace: {err}"));
        }
        return;
    }
    let mistral_home =
        match provider_accounts::ensure_mistral_runtime_home(&state.core.data_root).await {
            Ok(home) => home,
            Err(err) => {
                let mut map = state.providers.mistral_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    entry.error = Some(format!("failed to prepare mistral runtime home: {err}"));
                }
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }
        };

    let mut provider_env = HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    if let Some(token) = state.core.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.core.data_root.to_string_lossy().to_string(),
    );
    provider_env.insert(
        "HOME".to_string(),
        mistral_home.to_string_lossy().to_string(),
    );
    provider_env.insert(
        "XDG_CONFIG_HOME".to_string(),
        mistral_home.join(".config").to_string_lossy().to_string(),
    );
    provider_env.insert(
        "XDG_CACHE_HOME".to_string(),
        mistral_home.join(".cache").to_string_lossy().to_string(),
    );

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let auth_result = adapter
        .authenticate_session(
            format!("mistral-login-{login_id}"),
            workdir,
            provider_env,
            None,
            event_tx,
        )
        .await;
    if let Err(err) = auth_result {
        let mut map = state.providers.mistral_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = "failed".to_string();
            entry.error = Some(logs::redact_sensitive(&err.to_string()));
        }
        let _ = tokio::fs::remove_dir_all(&login_home).await;
        return;
    }

    let started_at = Instant::now();
    let timeout = mistral_login_timeout();
    let mut observed_auth_url = false;
    let mut observed_email = None::<String>;

    loop {
        if started_at.elapsed() >= timeout {
            let mut map = state.providers.mistral_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "timeout".to_string();
                if entry.error.is_none() {
                    entry.error =
                        Some("timed out waiting for Mistral OAuth completion".to_string());
                }
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        let event = match tokio::time::timeout(MISTRAL_LOGIN_POLL_INTERVAL, event_rx.recv()).await {
            Ok(Some(event)) => event,
            Ok(None) => {
                let mut map = state.providers.mistral_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    if entry.error.is_none() {
                        entry.error = Some(if observed_auth_url {
                            "Mistral sign-in session ended before completion.".to_string()
                        } else {
                            "Mistral sign-in did not emit an OAuth URL in this environment."
                                .to_string()
                        });
                    }
                }
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }
            Err(_) => continue,
        };

        if let Some(auth_url) = extract_auth_url_from_value(&event.payload_json) {
            observed_auth_url = true;
            let mut map = state.providers.mistral_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.auth_url = Some(auth_url);
            }
        }
        if observed_email.is_none() {
            observed_email = first_email_from_value(&event.payload_json);
        }

        if matches!(event.event_type, ctx_core::models::SessionEventType::Error) {
            let message = event
                .payload_json
                .get("message")
                .and_then(serde_json::Value::as_str)
                .map(logs::redact_sensitive)
                .unwrap_or_else(|| "mistral authenticate reported an error".to_string());
            let mut map = state.providers.mistral_login_sessions.lock().await;
            if let Some(entry) = map.get_mut(&login_id) {
                entry.status = "failed".to_string();
                entry.error = Some(message);
            }
            let _ = tokio::fs::remove_dir_all(&login_home).await;
            return;
        }

        if matches!(event.event_type, ctx_core::models::SessionEventType::Notice) {
            let code = auth_notice_code(&event.payload_json);
            if is_auth_success_notice_code(code) {
                if let Err(err) = provider_accounts::upsert_mistral_account(
                    &state.core.data_root,
                    label.clone(),
                    observed_email.clone(),
                )
                .await
                {
                    let mut map = state.providers.mistral_login_sessions.lock().await;
                    if let Some(entry) = map.get_mut(&login_id) {
                        entry.status = "failed".to_string();
                        entry.error = Some(logs::redact_sensitive(&err.to_string()));
                    }
                    let _ = tokio::fs::remove_dir_all(&login_home).await;
                    return;
                }
                let mut map = state.providers.mistral_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "success".to_string();
                    entry.auth_url = None;
                    entry.error = None;
                }
                restart_mistral_providers_for_auth_change(&state, "mistral auth updated").await;
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }
            if is_auth_failure_notice_code(code) {
                let message = event
                    .payload_json
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .map(logs::redact_sensitive)
                    .unwrap_or_else(|| "Mistral sign-in failed. Retry.".to_string());
                let mut map = state.providers.mistral_login_sessions.lock().await;
                if let Some(entry) = map.get_mut(&login_id) {
                    entry.status = "failed".to_string();
                    entry.error = Some(message);
                }
                let _ = tokio::fs::remove_dir_all(&login_home).await;
                return;
            }
        }
    }
}

pub(super) async fn start_mistral_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MistralLoginStartReq>,
) -> Result<Json<MistralLoginStartResp>, (StatusCode, Json<ApiErrorResp>)> {
    let label = req.label;
    let login_id = uuid::Uuid::new_v4().to_string();
    {
        let mut map = state.providers.mistral_login_sessions.lock().await;
        map.insert(
            login_id.clone(),
            provider_accounts::MistralLoginStatus {
                login_id: login_id.clone(),
                auth_url: None,
                status: "pending".to_string(),
                error: None,
            },
        );
    }

    let state_clone = Arc::clone(&state);
    let login_id_for_task = login_id.clone();
    tokio::spawn(async move {
        monitor_mistral_login(state_clone, login_id_for_task, label).await;
    });

    Ok(Json(MistralLoginStartResp {
        login_id,
        auth_url: None,
    }))
}

pub(super) async fn get_mistral_login(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<provider_accounts::MistralLoginStatus>, (StatusCode, Json<ApiErrorResp>)> {
    let map = state.providers.mistral_login_sessions.lock().await;
    let status = map.get(&id).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "login not found".to_string(),
            }),
        )
    })?;
    Ok(Json(status))
}

pub(super) async fn list_amp_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let response = amp_accounts_response(&state).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(Json(response))
}

pub(super) async fn set_amp_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AmpActiveAccountReq>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_amp_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_amp_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_amp_providers_for_auth_change(&state, "amp auth updated").await;
    let response = amp_accounts_response(&state).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(Json(response))
}

pub(super) async fn delete_amp_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<AmpAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_amp_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_amp_providers_for_auth_change(&state, "amp auth updated").await;
    let response = amp_accounts_response(&state).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(Json(response))
}

pub(super) async fn list_gemini_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(gemini_accounts_response(&state).await))
}

pub(super) async fn upsert_gemini_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GeminiAccountUpsertReq>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_gemini_account(
        &state.core.data_root,
        req.label,
        req.oauth_creds_json,
        req.google_accounts_json,
        req.email,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    restart_gemini_providers_for_auth_change(&state, "gemini auth updated").await;
    Ok(Json(gemini_accounts_response(&state).await))
}

pub(super) async fn set_gemini_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GeminiActiveAccountReq>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_gemini_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_gemini_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_gemini_providers_for_auth_change(&state, "gemini auth updated").await;
    Ok(Json(gemini_accounts_response(&state).await))
}

pub(super) async fn delete_gemini_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<GeminiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_gemini_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_gemini_providers_for_auth_change(&state, "gemini auth updated").await;
    Ok(Json(gemini_accounts_response(&state).await))
}

pub(super) async fn list_qwen_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(qwen_accounts_response(&state).await))
}

pub(super) async fn set_qwen_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QwenActiveAccountReq>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_qwen_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_qwen_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_qwen_providers_for_auth_change(&state, "qwen auth updated").await;
    Ok(Json(qwen_accounts_response(&state).await))
}

pub(super) async fn delete_qwen_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<QwenAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_qwen_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_qwen_providers_for_auth_change(&state, "qwen auth updated").await;
    Ok(Json(qwen_accounts_response(&state).await))
}

pub(super) async fn list_kimi_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(kimi_accounts_response(&state).await))
}

pub(super) async fn upsert_kimi_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<KimiAccountUpsertReq>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_kimi_account(
        &state.core.data_root,
        req.label,
        req.provider,
        req.credentials_json,
        req.config_toml,
        req.email,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    restart_kimi_providers_for_auth_change(&state, "kimi auth updated").await;
    Ok(Json(kimi_accounts_response(&state).await))
}

pub(super) async fn set_kimi_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<KimiActiveAccountReq>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_kimi_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_kimi_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_kimi_providers_for_auth_change(&state, "kimi auth updated").await;
    Ok(Json(kimi_accounts_response(&state).await))
}

pub(super) async fn delete_kimi_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<KimiAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_kimi_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_kimi_providers_for_auth_change(&state, "kimi auth updated").await;
    Ok(Json(kimi_accounts_response(&state).await))
}

pub(super) async fn list_mistral_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(mistral_accounts_response(&state).await))
}

pub(super) async fn set_mistral_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MistralActiveAccountReq>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_mistral_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_mistral_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_mistral_providers_for_auth_change(&state, "mistral auth updated").await;
    Ok(Json(mistral_accounts_response(&state).await))
}

pub(super) async fn delete_mistral_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<MistralAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_mistral_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_mistral_providers_for_auth_change(&state, "mistral auth updated").await;
    Ok(Json(mistral_accounts_response(&state).await))
}

pub(super) async fn list_copilot_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(copilot_accounts_response(&state).await))
}

pub(super) async fn upsert_copilot_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CopilotAccountUpsertReq>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_copilot_account(&state.core.data_root, req.label, req.token, req.email)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_copilot_providers_for_auth_change(&state, "copilot auth updated").await;
    Ok(Json(copilot_accounts_response(&state).await))
}

pub(super) async fn set_copilot_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CopilotActiveAccountReq>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_copilot_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_copilot_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_copilot_providers_for_auth_change(&state, "copilot auth updated").await;
    Ok(Json(copilot_accounts_response(&state).await))
}

pub(super) async fn delete_copilot_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<CopilotAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_copilot_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_copilot_providers_for_auth_change(&state, "copilot auth updated").await;
    Ok(Json(copilot_accounts_response(&state).await))
}

pub(super) async fn list_cursor_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(cursor_accounts_response(&state).await))
}

pub(super) async fn upsert_cursor_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CursorAccountUpsertReq>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_cursor_account(&state.core.data_root, req.label, req.token, req.email)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_cursor_providers_for_auth_change(&state, "cursor auth updated").await;
    Ok(Json(cursor_accounts_response(&state).await))
}

pub(super) async fn set_cursor_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CursorActiveAccountReq>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_cursor_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_cursor_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_cursor_providers_for_auth_change(&state, "cursor auth updated").await;
    Ok(Json(cursor_accounts_response(&state).await))
}

pub(super) async fn delete_cursor_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<CursorAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_cursor_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_cursor_providers_for_auth_change(&state, "cursor auth updated").await;
    Ok(Json(cursor_accounts_response(&state).await))
}

pub(super) async fn list_provider_auth_import_candidates(
    _state: State<Arc<AppState>>,
) -> Result<Json<ProviderAuthImportCandidatesResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let candidates = provider_auth_import::list_provider_auth_import_candidates()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok(Json(ProviderAuthImportCandidatesResponse { candidates }))
}

pub(super) async fn list_provider_auth_import_profiles(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ProviderAuthImportProfilesResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let profiles = provider_auth_import::list_provider_auth_profiles(&state.core.data_root)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok(Json(ProviderAuthImportProfilesResponse { profiles }))
}

pub(super) async fn import_provider_auth_candidates(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ProviderAuthImportReq>,
) -> Result<Json<ProviderAuthImportResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let mut ids = Vec::new();
    for id in req.candidate_ids {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            ids.push(trimmed.to_string());
        }
    }
    let results =
        provider_auth_import::import_provider_auth_candidates(&state.core.data_root, &ids)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: e.to_string(),
                    }),
                )
            })?;
    let mutated_providers: HashSet<String> = results
        .iter()
        .filter(|result| import_result_requires_provider_restart(result))
        .map(|result| result.provider_id.clone())
        .collect();
    for provider_id in mutated_providers {
        restart_provider_for_auth_change(
            &state,
            &provider_id,
            &format!("{provider_id} auth updated"),
        )
        .await;
    }
    Ok(Json(ProviderAuthImportResponse { results }))
}

async fn resolve_runtime_provider_command_from_config(
    data_root: &std::path::Path,
    provider_id: &str,
) -> anyhow::Result<Option<installer::ProviderRuntimeCommand>> {
    let cfg = installer::load_agent_server_config(data_root)
        .await
        .context("loading agent server config")?;
    installer::resolve_runtime_provider_command(&cfg, provider_id)
        .with_context(|| format!("resolving runtime command for {provider_id}"))
}

fn should_attempt_claude_cli_bootstrap(
    resolution: &anyhow::Result<Option<installer::ProviderRuntimeCommand>>,
) -> bool {
    match resolution {
        Ok(Some(_)) => false,
        Ok(None) => true,
        Err(err) => {
            let msg = err.to_string();
            msg.contains("runtime_command_missing")
                || msg.contains("runtime_command_not_found")
                || msg.contains("runtime_command_not_absolute")
        }
    }
}

async fn resolve_claude_setup_token_runtime_with_bootstrap<F, Fut>(
    data_root: &std::path::Path,
    bootstrap_managed_runtime: F,
) -> anyhow::Result<installer::ProviderRuntimeCommand>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let first_resolution =
        resolve_runtime_provider_command_from_config(data_root, "claude-cli").await;
    if let Ok(Some(runtime_command)) = first_resolution.as_ref() {
        return Ok(runtime_command.clone());
    }

    if should_attempt_claude_cli_bootstrap(&first_resolution) {
        bootstrap_managed_runtime()
            .await
            .context("installing managed claude-cli runtime for subscription login")?;
        let resolved = resolve_runtime_provider_command_from_config(data_root, "claude-cli")
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "runtime_command_missing: provider=claude-cli (bundle claude-cli or configure an absolute runtime command)"
                )
            })?;
        return Ok(resolved);
    }

    match first_resolution {
        Ok(Some(runtime_command)) => Ok(runtime_command),
        Ok(None) => anyhow::bail!(
            "runtime_command_missing: provider=claude-cli (bundle claude-cli or configure an absolute runtime command)"
        ),
        Err(err) => Err(err),
    }
}

async fn resolve_claude_setup_token_runtime(
    state: &Arc<AppState>,
) -> anyhow::Result<installer::ProviderRuntimeCommand> {
    resolve_claude_setup_token_runtime_with_bootstrap(&state.core.data_root, || async {
        installer::install_provider(state.as_ref(), "claude-cli").await
    })
    .await
}

fn spawn_claude_setup_token_command(
    runtime: &installer::ProviderRuntimeCommand,
) -> anyhow::Result<ClaudeLoginSpawn> {
    let pty = NativePtySystem::default();
    let pair = pty
        .openpty(PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("opening pty for claude setup-token")?;

    let mut cmd = CommandBuilder::new(&runtime.command_abs_path);
    for arg in &runtime.args {
        cmd.arg(arg);
    }
    cmd.arg("setup-token");
    cmd.env("NO_COLOR", "1");
    cmd.env("TERM", "xterm-256color");

    let mut child = pair.slave.spawn_command(cmd).with_context(|| {
        format!(
            "spawning claude setup-token via {}",
            runtime.command_abs_path
        )
    })?;
    let killer = Arc::new(StdMutex::new(child.clone_killer()));
    drop(pair.slave);
    let writer = Arc::new(StdMutex::new(
        pair.master
            .take_writer()
            .context("taking pty writer for claude setup-token")?,
    ));

    let reader = pair
        .master
        .try_clone_reader()
        .context("cloning pty reader for claude setup-token")?;
    let (line_tx, line_rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        pump_claude_login_output(reader, line_tx);
    });

    let (exit_tx, exit_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let result = child
            .wait()
            .context("waiting for claude setup-token process");
        let _ = exit_tx.send(result);
    });

    Ok(ClaudeLoginSpawn {
        line_rx,
        exit_rx,
        killer,
        writer,
    })
}

fn strip_ansi_sequences(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut idx = 0usize;
    while idx < chars.len() {
        let ch = chars[idx];
        if ch == '\u{1b}' {
            idx += 1;
            if idx < chars.len() {
                if chars[idx] == '[' {
                    idx += 1;
                    while idx < chars.len() {
                        let c = chars[idx];
                        idx += 1;
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                    continue;
                }
                if chars[idx] == ']' {
                    idx += 1;
                    let mut payload = String::new();
                    while idx < chars.len() {
                        let c = chars[idx];
                        if c == '\u{7}' {
                            idx += 1;
                            break;
                        }
                        if c == '\u{1b}' && (idx + 1) < chars.len() && chars[idx + 1] == '\\' {
                            idx += 2;
                            break;
                        }
                        payload.push(c);
                        idx += 1;
                    }
                    if let Some(url) = payload
                        .strip_prefix("8;;")
                        .filter(|value| !value.is_empty())
                    {
                        out.push(' ');
                        out.push_str(url);
                        out.push(' ');
                    }
                    continue;
                }
            }
            continue;
        }
        if ch != '\r' && ch != '\u{7}' {
            out.push(ch);
        }
        idx += 1;
    }
    out
}

fn normalize_claude_login_line(line: &str) -> String {
    strip_ansi_sequences(line.trim_end_matches('\r'))
}

fn pump_claude_login_output<R>(mut reader: R, tx: mpsc::UnboundedSender<String>)
where
    R: std::io::Read,
{
    let mut pending = String::new();
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                while let Some(newline_idx) = pending.find('\n') {
                    let raw = pending[..newline_idx].to_string();
                    pending.drain(..=newline_idx);
                    if tx.send(normalize_claude_login_line(&raw)).is_err() {
                        return;
                    }
                }
            }
            Err(_) => break,
        }
    }
    if !pending.is_empty() {
        let _ = tx.send(normalize_claude_login_line(&pending));
    }
}

fn is_auth_url_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '-' | '.'
                | '_'
                | '~'
                | ':'
                | '/'
                | '?'
                | '#'
                | '['
                | ']'
                | '@'
                | '!'
                | '$'
                | '&'
                | '\''
                | '('
                | ')'
                | '*'
                | '+'
                | ','
                | ';'
                | '='
                | '%'
        )
}

fn should_continue_auth_url_after_break(current: &str, next_fragment: &str) -> bool {
    if next_fragment.is_empty() {
        return false;
    }
    if next_fragment.chars().any(|ch| !ch.is_ascii_alphanumeric()) {
        return true;
    }
    if next_fragment
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
    {
        return true;
    }
    matches!(
        current.chars().last(),
        Some('%' | ':' | '=' | '&' | '?' | '/' | '#' | '-' | '_')
    )
}

fn extract_auth_url(text: &str) -> Option<String> {
    let normalized = strip_ansi_sequences(text);
    let chars: Vec<char> = normalized.chars().collect();
    let mut idx = 0usize;
    while idx < chars.len() {
        let remaining: String = chars[idx..].iter().collect();
        let scheme_offset = remaining
            .find("https://")
            .or_else(|| remaining.find("http://"))?;
        idx += scheme_offset;
        let mut end = idx;
        let mut candidate = String::new();
        while end < chars.len() {
            let ch = chars[end];
            if is_auth_url_char(ch) {
                candidate.push(ch);
                end += 1;
                continue;
            }
            if ch.is_whitespace() {
                let mut probe = end;
                while probe < chars.len() && chars[probe].is_whitespace() {
                    probe += 1;
                }
                if probe < chars.len() && is_auth_url_char(chars[probe]) {
                    let mut token_end = probe;
                    while token_end < chars.len() && is_auth_url_char(chars[token_end]) {
                        token_end += 1;
                    }
                    let next_fragment: String = chars[probe..token_end].iter().collect();
                    if should_continue_auth_url_after_break(&candidate, &next_fragment) {
                        end = probe;
                        continue;
                    }
                }
            }
            break;
        }
        let trimmed = candidate
            .trim_matches(|c: char| {
                matches!(
                    c,
                    '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';' | '.'
                )
            })
            .to_string();
        if Url::parse(&trimmed).is_ok() {
            return Some(trimmed);
        }
        idx = end.saturating_add(1);
    }
    None
}

fn auth_url_looks_complete(auth_url: &str) -> bool {
    let parsed = match Url::parse(auth_url) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let maybe_redirect = parsed
        .query_pairs()
        .find_map(|(key, value)| (key == "redirect_uri").then_some(value.into_owned()));
    let Some(redirect_uri) = maybe_redirect else {
        return true;
    };
    let redirect = match Url::parse(&redirect_uri) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let Some(host) = redirect.host_str() else {
        return false;
    };
    if !is_loopback_host(host) {
        return true;
    }
    redirect.port().is_some()
}

fn is_claude_setup_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'
}

fn is_setup_token_fragment(value: &str) -> bool {
    !value.is_empty() && value.chars().all(is_claude_setup_token_char)
}

fn leading_setup_token_fragment(value: &str) -> &str {
    let mut end = 0usize;
    for (idx, ch) in value.char_indices() {
        if is_claude_setup_token_char(ch) {
            end = idx + ch.len_utf8();
            continue;
        }
        break;
    }
    &value[..end]
}

fn is_setup_token_continuation_fragment(value: &str) -> bool {
    if !is_setup_token_fragment(value) {
        return false;
    }
    value
        .chars()
        .any(|ch| ch.is_ascii_digit() || ch == '-' || ch == '_')
}

fn trim_known_setup_token_prose_suffix(token: &str) -> String {
    const PROSE_CANONICAL: &str = "StorethistokensecurelyYouwontbeabletoseeitagain";
    const MIN_MATCH_LEN: usize = 5;

    let mut out = token.to_string();
    for phrase in [
        PROSE_CANONICAL,
        "storethistokensecurelyyouwontbeabletoseeitagain",
    ] {
        let max = std::cmp::min(out.len(), phrase.len());
        let mut truncate_at: Option<usize> = None;
        for len in (MIN_MATCH_LEN..=max).rev() {
            if out.ends_with(&phrase[..len]) {
                truncate_at = Some(out.len() - len);
                break;
            }
        }
        if let Some(idx) = truncate_at {
            out.truncate(idx);
        }
    }
    out
}

fn extract_claude_setup_token(output: &str) -> Option<String> {
    let lines: Vec<&str> = output.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let Some(start) = line.find("sk-ant-oat") else {
            continue;
        };
        let first_fragment = leading_setup_token_fragment(&line[start..]);
        if first_fragment.is_empty() {
            continue;
        }
        let mut token = first_fragment.to_string();
        for next in lines.iter().skip(idx + 1) {
            let trimmed = next.trim();
            if trimmed.is_empty() {
                break;
            }
            let fragment = leading_setup_token_fragment(trimmed);
            if !is_setup_token_continuation_fragment(fragment) {
                break;
            }
            token.push_str(fragment);
        }
        let cleaned = trim_known_setup_token_prose_suffix(&token);
        if cleaned.len() > 40 {
            return Some(cleaned);
        }
    }
    None
}

pub(super) async fn start_claude_login_process(
    state: &Arc<AppState>,
) -> anyhow::Result<ClaudeLoginProcess> {
    let runtime = resolve_claude_setup_token_runtime(state).await?;
    let ClaudeLoginSpawn {
        line_rx: mut rx,
        exit_rx,
        killer,
        writer,
    } = spawn_claude_setup_token_command(&runtime)?;

    let mut buffered_lines = Vec::new();
    let mut auth_url = None;
    let mut transcript = String::new();
    let hard_deadline = Instant::now() + CLAUDE_LOGIN_URL_WAIT;
    let mut settle_deadline: Option<Instant> = None;
    loop {
        let now = Instant::now();
        let remaining = if let Some(settle) = settle_deadline {
            std::cmp::min(
                hard_deadline.saturating_duration_since(now),
                settle.saturating_duration_since(now),
            )
        } else {
            hard_deadline.saturating_duration_since(now)
        };
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(line)) => {
                transcript.push_str(&line);
                transcript.push('\n');
                buffered_lines.push(line);
                if let Some(candidate) = extract_auth_url(&transcript) {
                    auth_url = Some(candidate);
                    settle_deadline = Some(Instant::now() + CLAUDE_LOGIN_URL_SETTLE_WAIT);
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    Ok(ClaudeLoginProcess {
        line_rx: rx,
        input_rx: {
            let (_tx, rx) = mpsc::unbounded_channel();
            rx
        },
        buffered_lines,
        auth_url,
        exit_rx,
        killer,
        writer,
    })
}

async fn append_claude_login_line(
    state: &Arc<AppState>,
    login_id: &str,
    observed_auth_url: &mut Option<String>,
    transcript: &mut String,
    line: String,
) {
    let needs_auth_url_upgrade = match observed_auth_url.as_deref() {
        None => true,
        Some(url) => !auth_url_looks_complete(url),
    };
    if needs_auth_url_upgrade {
        if let Some(candidate) = extract_auth_url(&line) {
            let should_replace = match observed_auth_url.as_ref() {
                None => true,
                Some(current) => {
                    !auth_url_looks_complete(current) && candidate.len() >= current.len()
                }
            };
            if should_replace {
                *observed_auth_url = Some(candidate);
            }
        }
    }
    transcript.push_str(&line);
    transcript.push('\n');
    let needs_auth_url_upgrade = match observed_auth_url.as_deref() {
        None => true,
        Some(url) => !auth_url_looks_complete(url),
    };
    if needs_auth_url_upgrade {
        if let Some(candidate) = extract_auth_url(transcript) {
            let should_replace = match observed_auth_url.as_ref() {
                None => true,
                Some(current) => {
                    !auth_url_looks_complete(current) && candidate.len() >= current.len()
                }
            };
            if should_replace {
                *observed_auth_url = Some(candidate);
            }
        }
    }
    if let Some(url) = observed_auth_url.clone() {
        let mut map = state.providers.claude_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(login_id) {
            entry.auth_url = Some(url);
        }
    }
}

fn format_claude_exit_status(status: &portable_pty::ExitStatus) -> String {
    if let Some(signal) = status.signal() {
        return format!("signal {signal}");
    }
    status.exit_code().to_string()
}

async fn kill_claude_login_process(
    killer: Arc<StdMutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut guard = killer
            .lock()
            .map_err(|_| anyhow::anyhow!("claude setup-token killer lock poisoned"))?;
        guard.kill().context("killing claude setup-token process")
    })
    .await
    .context("joining claude setup-token kill task")?
}

async fn write_claude_login_input(
    writer: Arc<StdMutex<Box<dyn Write + Send>>>,
    input: &str,
) -> anyhow::Result<()> {
    let mut payload = input.trim().to_string();
    if payload.is_empty() {
        bail!("callback code is required");
    }
    payload.push('\n');
    tokio::task::spawn_blocking(move || {
        let mut guard = writer
            .lock()
            .map_err(|_| anyhow::anyhow!("claude setup-token writer lock poisoned"))?;
        guard
            .write_all(payload.as_bytes())
            .context("writing callback code to claude setup-token")?;
        guard
            .flush()
            .context("flushing callback code to claude setup-token")
    })
    .await
    .context("joining callback input writer task")?
}

async fn read_trailing_claude_login_lines(
    line_rx: &mut mpsc::UnboundedReceiver<String>,
    grace: Duration,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut deadline = Instant::now() + grace;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, line_rx.recv()).await {
            Ok(Some(line)) => {
                lines.push(line);
                deadline = Instant::now() + grace;
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    lines
}

pub(super) async fn monitor_claude_login(
    state: Arc<AppState>,
    login_id: String,
    label: Option<String>,
    mut login: ClaudeLoginProcess,
) {
    let mut transcript = String::new();
    let mut observed_auth_url = login.auth_url.clone();
    let auth_url_deadline = Instant::now() + CLAUDE_LOGIN_NO_AUTH_URL_TIMEOUT;
    let mut completion_deadline = observed_auth_url
        .as_ref()
        .map(|_| Instant::now() + CLAUDE_LOGIN_COMPLETION_TIMEOUT);

    for line in std::mem::take(&mut login.buffered_lines) {
        let had_auth_url = observed_auth_url.is_some();
        append_claude_login_line(
            &state,
            &login_id,
            &mut observed_auth_url,
            &mut transcript,
            line,
        )
        .await;
        if !had_auth_url && observed_auth_url.is_some() {
            completion_deadline = Some(Instant::now() + CLAUDE_LOGIN_COMPLETION_TIMEOUT);
        }
    }

    let mut exit_result: Option<anyhow::Result<portable_pty::ExitStatus>> = None;
    let mut timeout_error: Option<String> = None;

    loop {
        let deadline = completion_deadline.unwrap_or(auth_url_deadline);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            timeout_error = Some(if observed_auth_url.is_some() {
                "claude setup-token timed out waiting for browser sign-in completion".to_string()
            } else {
                "claude setup-token did not emit an authentication URL".to_string()
            });
            break;
        }
        let timeout_future = tokio::time::sleep(remaining);
        tokio::pin!(timeout_future);

        tokio::select! {
            maybe_line = login.line_rx.recv() => {
                match maybe_line {
                    Some(line) => {
                        let had_auth_url = observed_auth_url.is_some();
                        append_claude_login_line(
                            &state,
                            &login_id,
                            &mut observed_auth_url,
                            &mut transcript,
                            line,
                        )
                        .await;
                        if !had_auth_url && observed_auth_url.is_some() {
                            completion_deadline = Some(Instant::now() + CLAUDE_LOGIN_COMPLETION_TIMEOUT);
                        }
                    }
                    None => {
                        match tokio::time::timeout(CLAUDE_LOGIN_EXIT_GRACE_WAIT, &mut login.exit_rx).await {
                            Ok(Ok(result)) => {
                                exit_result = Some(result);
                            }
                            Ok(Err(err)) => {
                                exit_result = Some(Err(anyhow::anyhow!("claude setup-token exit channel closed: {err}")));
                            }
                            Err(_) => {
                                timeout_error = Some(
                                    "claude setup-token output stream closed before process exit".to_string(),
                                );
                            }
                        }
                        break;
                    }
                }
            }
            exit = &mut login.exit_rx => {
                exit_result = Some(match exit {
                    Ok(result) => result,
                    Err(err) => Err(anyhow::anyhow!("claude setup-token exit channel closed: {err}")),
                });
                break;
            }
            maybe_input = login.input_rx.recv() => {
                if let Some(input) = maybe_input {
                    if let Err(err) = write_claude_login_input(Arc::clone(&login.writer), &input).await {
                        timeout_error = Some(format!("failed to submit callback code to claude setup-token: {err}"));
                        break;
                    }
                }
            }
            _ = &mut timeout_future => {
                timeout_error = Some(if observed_auth_url.is_some() {
                    "claude setup-token timed out waiting for browser sign-in completion".to_string()
                } else {
                    "claude setup-token did not emit an authentication URL".to_string()
                });
                break;
            }
        }
    }

    if exit_result.is_some() {
        // PTY reader runs on a separate thread, so process exit can arrive before final
        // buffered lines. Keep consuming briefly before token parsing.
        for line in
            read_trailing_claude_login_lines(&mut login.line_rx, CLAUDE_LOGIN_EXIT_GRACE_WAIT).await
        {
            append_claude_login_line(
                &state,
                &login_id,
                &mut observed_auth_url,
                &mut transcript,
                line,
            )
            .await;
        }
    } else {
        while let Ok(line) = login.line_rx.try_recv() {
            append_claude_login_line(
                &state,
                &login_id,
                &mut observed_auth_url,
                &mut transcript,
                line,
            )
            .await;
        }
    }

    if timeout_error.is_some() {
        if let Err(err) = kill_claude_login_process(Arc::clone(&login.killer)).await {
            let suffix = format!("; failed to terminate setup-token process cleanly: {err}");
            timeout_error = Some(match timeout_error.take() {
                Some(base) => format!("{base}{suffix}"),
                None => suffix,
            });
        }
        if exit_result.is_none() {
            if let Ok(exit) =
                tokio::time::timeout(CLAUDE_LOGIN_EXIT_GRACE_WAIT, &mut login.exit_rx).await
            {
                exit_result = Some(match exit {
                    Ok(result) => result,
                    Err(err) => Err(anyhow::anyhow!(
                        "claude setup-token exit channel closed: {err}"
                    )),
                });
            }
        }
    }

    let mut final_status = "failed".to_string();
    let mut final_error: Option<String> = timeout_error;
    let mut final_account_id: Option<String> = None;

    if final_error.is_none() {
        match exit_result {
            Some(Ok(exit)) if exit.success() => match extract_claude_setup_token(&transcript) {
                Some(setup_token) => {
                    match provider_accounts::add_claude_account(
                        &state.core.data_root,
                        label.clone(),
                        setup_token,
                    )
                    .await
                    {
                        Ok(registry) => {
                            final_status = "success".to_string();
                            final_account_id = registry.active_account_id;
                            restart_claude_providers_for_auth_change(&state, "claude auth updated")
                                .await;
                        }
                        Err(err) => {
                            final_error = Some(logs::redact_sensitive(&err.to_string()));
                        }
                    }
                }
                None => {
                    final_error = Some(
                        "claude setup-token completed but no setup token was detected".to_string(),
                    );
                }
            },
            Some(Ok(exit)) => {
                final_error = Some(format!(
                    "claude setup-token exited with status {}",
                    format_claude_exit_status(&exit)
                ));
            }
            Some(Err(err)) => {
                final_error = Some(format!("waiting for claude setup-token failed: {err}"));
            }
            None => {
                final_error = Some(
                    "claude setup-token monitor ended before process exit was observed".to_string(),
                );
            }
        }
    }

    {
        let mut map = state.providers.claude_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&login_id) {
            entry.status = final_status;
            entry.account_id = final_account_id;
            entry.error = final_error;
            if entry.auth_url.is_none() {
                entry.auth_url = observed_auth_url;
            }
        }
    }
    {
        let mut map = state.providers.claude_login_inputs.lock().await;
        map.remove(&login_id);
    }
}

pub(super) async fn start_codex_login_process(
    account_dir: &PathBuf,
) -> anyhow::Result<CodexLoginProcess> {
    let mut child = spawn_codex_app_server(account_dir)?;
    let stdout = child
        .stdout
        .take()
        .context("codex app-server stdout unavailable")?;
    let mut reader = BufReader::new(stdout).lines();
    let mut stdin = child
        .stdin
        .take()
        .context("codex app-server stdin unavailable")?;

    let init_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "ctx",
                "title": "ctx",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    });
    send_codex_jsonrpc(&mut stdin, &init_request).await?;
    wait_for_codex_response(&mut reader, 1, CODEX_LOGIN_RPC_TIMEOUT).await?;
    send_codex_jsonrpc(
        &mut stdin,
        &serde_json::json!({"jsonrpc": "2.0", "method": "initialized"}),
    )
    .await?;

    let login_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "account/login/start",
        "params": { "type": "chatgpt" }
    });
    send_codex_jsonrpc(&mut stdin, &login_request).await?;
    let response = wait_for_codex_response(&mut reader, 2, CODEX_LOGIN_RPC_TIMEOUT).await?;
    let result = response
        .get("result")
        .and_then(|v| v.as_object())
        .context("codex login missing result")?;
    let auth_url = result
        .get("authUrl")
        .or_else(|| result.get("auth_url"))
        .and_then(|v| v.as_str())
        .context("codex login missing auth_url")?
        .to_string();
    let login_id = result
        .get("loginId")
        .or_else(|| result.get("login_id"))
        .and_then(|v| v.as_str())
        .context("codex login missing login_id")?
        .to_string();

    Ok(CodexLoginProcess {
        login_id,
        auth_url,
        account_dir: account_dir.clone(),
        child,
        stdin,
        reader,
    })
}

pub(super) async fn monitor_codex_login(
    state: Arc<AppState>,
    account_id: String,
    label: String,
    mut login: CodexLoginProcess,
) {
    let completion = wait_for_codex_login_completion(&mut login.reader, &login.login_id).await;
    let status = match completion {
        Ok(completion) => completion,
        Err(err) => CodexLoginCompletion {
            success: false,
            error: Some(err.to_string()),
        },
    };

    if status.success {
        let (email, plan_type) = fetch_codex_account_details(&mut login.stdin, &mut login.reader)
            .await
            .unwrap_or((None, None));
        let entry = provider_accounts::CodexAccountEntry {
            id: account_id.clone(),
            label,
            kind: provider_accounts::CODEX_CREDENTIAL_KIND_OAUTH.to_string(),
            email,
            plan_type,
            created_at: Utc::now(),
            last_used_at: Some(Utc::now()),
            secret_ref: None,
            endpoint_profile: provider_accounts::CodexEndpointProfile::default(),
        };
        if provider_accounts::upsert_codex_account(&state.core.data_root, entry)
            .await
            .is_ok()
        {
            let _ = provider_accounts::ingest_codex_account_auth_to_secret_store(
                &state.core.data_root,
                &account_id,
            )
            .await;
            let _ = provider_accounts::set_active_codex_account(
                &state.core.data_root,
                Some(account_id.clone()),
            )
            .await;
            restart_codex_providers_for_auth_change(&state, "codex auth updated").await;
        }
    } else {
        let _ = tokio::fs::remove_dir_all(&login.account_dir).await;
    }

    {
        let mut map = state.providers.codex_login_sessions.lock().await;
        if let Some(entry) = map.get_mut(&account_id) {
            entry.status = if status.success {
                "success".to_string()
            } else {
                "failed".to_string()
            };
            entry.completion_token = None;
            entry.error = status.error;
        }
    }

    let _ = login.child.kill().await;
}

fn spawn_codex_app_server(account_dir: &PathBuf) -> anyhow::Result<tokio::process::Child> {
    let mut cmd = Command::new("codex");
    cmd.args(["-s", "read-only", "-a", "untrusted", "app-server"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd.env("CODEX_HOME", account_dir);
    cmd.spawn().context("spawning codex app-server")
}

pub(super) async fn send_codex_jsonrpc(
    stdin: &mut tokio::process::ChildStdin,
    value: &serde_json::Value,
) -> anyhow::Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    stdin.write_all(&bytes).await?;
    stdin.flush().await?;
    Ok(())
}

pub(super) async fn wait_for_codex_response(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    request_id: i64,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("codex rpc timeout waiting for response");
        }
        let line = tokio::time::timeout(remaining, reader.next_line())
            .await
            .context("codex rpc read timeout")??;
        let line = line.ok_or_else(|| anyhow::anyhow!("codex rpc stdout closed"))?;
        let value: serde_json::Value = serde_json::from_str(&line)?;
        if let Some(id) = value.get("id").and_then(|v| v.as_i64()) {
            if id == request_id {
                if value.get("error").is_some() {
                    bail!("codex rpc error: {value}");
                }
                return Ok(value);
            }
        }
    }
}

pub(super) async fn wait_for_codex_login_completion(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    login_id: &str,
) -> anyhow::Result<CodexLoginCompletion> {
    loop {
        let line = reader
            .next_line()
            .await
            .context("codex login read failed")?;
        let line = line.ok_or_else(|| anyhow::anyhow!("codex login stdout closed"))?;
        let value: serde_json::Value = serde_json::from_str(&line)?;
        let method = value.get("method").and_then(|v| v.as_str());
        if method != Some("account/login/completed") {
            continue;
        }
        let params = value.get("params").unwrap_or(&serde_json::Value::Null);
        let found_login_id = params
            .get("loginId")
            .or_else(|| params.get("login_id"))
            .and_then(|v| v.as_str());
        if let Some(found) = found_login_id {
            if found != login_id {
                continue;
            }
        }
        let success = params
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let error = params
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        return Ok(CodexLoginCompletion { success, error });
    }
}

pub(super) async fn fetch_codex_account_details(
    stdin: &mut tokio::process::ChildStdin,
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "account/read",
        "params": { "refreshToken": false }
    });
    send_codex_jsonrpc(stdin, &request).await?;
    let response = wait_for_codex_response(reader, 3, CODEX_LOGIN_RPC_TIMEOUT).await?;
    let account = response
        .get("result")
        .and_then(|v| v.get("account"))
        .and_then(|v| v.as_object());
    let Some(account) = account else {
        return Ok((None, None));
    };
    if account.get("type").and_then(|v| v.as_str()) != Some("chatgpt") {
        return Ok((None, None));
    }
    let email = account
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let plan_type = account
        .get("planType")
        .or_else(|| account.get("plan_type"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok((email, plan_type))
}

#[derive(Debug, Serialize)]
pub(super) struct InstallStartResponse {
    provider_id: String,
    install_id: InstallId,
    target: InstallTarget,
}

#[derive(Debug, Deserialize)]
pub(super) struct AuthenticateProviderReq {
    #[serde(default)]
    method_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SelectHarnessSourceReq {
    source_kind: HarnessSourceKind,
    #[serde(default)]
    endpoint_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpsertHarnessEndpointReq {
    #[serde(default)]
    endpoint_id: Option<String>,
    name: String,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    api_shape: Option<HarnessApiShape>,
    #[serde(default)]
    auth_type: Option<String>,
    #[serde(default)]
    model_override: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    manual_model_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetEndpointManualModelsReq {
    #[serde(default)]
    model_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ProviderAuthCheckResp {
    provider_id: String,
    workspace_id: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

fn parse_workspace_id(ws_id: &str) -> Result<WorkspaceId, (StatusCode, Json<serde_json::Value>)> {
    Ok(WorkspaceId(uuid::Uuid::parse_str(ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid workspace id",
            })),
        )
    })?))
}

pub(super) async fn get_workspace_providers_bootstrap(
    State(state): State<Arc<AppState>>,
    Path(ws_id): Path<String>,
) -> Result<Json<ProvidersBootstrapResponse>, (StatusCode, Json<serde_json::Value>)> {
    let ws_id = parse_workspace_id(&ws_id)?;

    let workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "failed to load workspace",
                })),
            )
        })?;
    if workspace.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "workspace not found",
            })),
        ));
    }

    let install_target = match execution_effective::effective_execution_settings(
        state.as_ref(),
        ws_id,
    )
    .await
    {
        Ok(effective) if matches!(effective.mode, ExecutionMode::Container) => {
            InstallTarget::Container
        }
        Ok(_) => InstallTarget::Host,
        Err(error) => {
            tracing::warn!(
                    "providers bootstrap falling back to host install target for workspace {}: {error:#}",
                    ws_id.0
                );
            InstallTarget::Host
        }
    };

    let providers = providers_statuses_response(&state, install_target).await;
    let mut provider_options = HashMap::new();
    let mut provider_harness_config = HashMap::new();
    let ws_id_str = ws_id.0.to_string();
    let visible_provider_ids = providers
        .iter()
        .filter(|provider| provider.details.get("ui_hidden").map(String::as_str) != Some("true"))
        .map(|provider| provider.provider_id.clone())
        .collect::<Vec<_>>();

    let per_provider = futures::stream::iter(visible_provider_ids.into_iter().map(|provider_id| {
        let state = Arc::clone(&state);
        let ws_id = ws_id_str.clone();
        async move {
            let source_config =
                harness_sources::get_provider_source_config(&state.core.data_root, &provider_id)
                    .await
                    .ok();
            let has_active_auth = provider_has_active_auth_config(
                &state.core.data_root,
                &provider_id,
                source_config.as_ref(),
            )
            .await;
            let auth_mode = provider_auth_mode(has_active_auth, source_config.as_ref());

            let mut options = serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id,
                "supports_load": false,
                "auth_required": false,
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "probe_ok": true,
                "probed_at": chrono::Utc::now().to_rfc3339(),
            });
            if let Some(source) = source_config.as_ref() {
                options["source"] = serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
            }

            (provider_id, options, source_config)
        }
    }))
    .buffer_unordered(visible_provider_count_hint(providers.len()))
    .collect::<Vec<_>>()
    .await;

    for (provider_id, options, source_config) in per_provider {
        provider_options.insert(provider_id.clone(), options);
        if let Some(config) = source_config {
            provider_harness_config.insert(provider_id, config);
        }
    }

    Ok(Json(ProvidersBootstrapResponse {
        providers,
        provider_options,
        provider_harness_config,
        codex_accounts: codex_accounts_response(&state).await,
        claude_accounts: claude_accounts_response(&state).await,
        gemini_accounts: gemini_accounts_response(&state).await,
        qwen_accounts: qwen_accounts_response(&state).await,
        kimi_accounts: kimi_accounts_response(&state).await,
        mistral_accounts: mistral_accounts_response(&state).await,
        copilot_accounts: copilot_accounts_response(&state).await,
        cursor_accounts: cursor_accounts_response(&state).await,
        amp_accounts: amp_accounts_response(&state).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("failed to load amp accounts: {}", logs::redact_sensitive(&e.to_string())),
                })),
            )
        })?,
    }))
}

fn visible_provider_count_hint(total_provider_count: usize) -> usize {
    total_provider_count.max(1)
}

fn classify_probe_error(
    message: &str,
) -> (
    &'static str,
    Option<bool>,
    HarnessEndpointVerificationStatus,
) {
    let lower = message.to_ascii_lowercase();
    if lower.contains("401")
        || lower.contains("403")
        || lower.contains("unauthorized")
        || lower.contains("auth")
        || lower.contains("api key")
        || lower.contains("token")
    {
        return (
            "auth_required",
            Some(true),
            HarnessEndpointVerificationStatus::Invalid,
        );
    }
    if lower.contains("timeout")
        || lower.contains("connection refused")
        || lower.contains("network")
        || lower.contains("econn")
        || lower.contains("dns")
        || lower.contains("tls")
    {
        return (
            "network_error",
            Some(false),
            HarnessEndpointVerificationStatus::Error,
        );
    }
    (
        "error",
        Some(false),
        HarnessEndpointVerificationStatus::Error,
    )
}

fn cache_key_matches_provider(cache_key: &str, provider_id: &str) -> bool {
    cache_key
        .rsplit_once('/')
        .is_some_and(|(_, key_provider)| key_provider == provider_id)
}

async fn invalidate_provider_probe_caches(state: &Arc<AppState>, provider_id: &str) {
    state
        .providers
        .options_cache
        .lock()
        .await
        .retain(|cache_key, _| !cache_key_matches_provider(cache_key, provider_id));
    state
        .providers
        .verify_cache
        .lock()
        .await
        .retain(|cache_key, _| !cache_key_matches_provider(cache_key, provider_id));
}

fn selected_endpoint_from_harness_config(
    config: Option<harness_sources::HarnessProviderSourceConfig>,
) -> Option<String> {
    config.and_then(|cfg| {
        if cfg.selected_source_kind == HarnessSourceKind::Endpoint {
            cfg.selected_endpoint_id
        } else {
            None
        }
    })
}

fn selected_endpoint_record_from_harness_config(
    config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> Option<harness_sources::HarnessEndpointRecord> {
    let cfg = config?;
    if cfg.selected_source_kind != HarnessSourceKind::Endpoint {
        return None;
    }
    let selected_id = cfg.selected_endpoint_id.as_deref()?;
    cfg.endpoints
        .iter()
        .find(|endpoint| endpoint.id == selected_id)
        .cloned()
}

fn droid_model_id_for_endpoint_model_override(model_override: Option<&str>) -> Option<String> {
    let model = model_override?.trim();
    if model.is_empty() {
        return None;
    }
    if model.starts_with("custom:") {
        return Some(model.to_string());
    }
    Some(format!("custom:{model}"))
}

fn endpoint_current_model_id(
    provider_id: &str,
    endpoint: &harness_sources::HarnessEndpointRecord,
) -> Option<String> {
    if provider_id == "droid" {
        return droid_model_id_for_endpoint_model_override(endpoint.model_override.as_deref());
    }
    endpoint
        .model_override
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn endpoint_models_payload(
    provider_id: &str,
    endpoint: &harness_sources::HarnessEndpointRecord,
    now: chrono::DateTime<chrono::Utc>,
) -> serde_json::Value {
    let stale = harness_sources::endpoint_model_catalog_is_stale(endpoint, now);
    serde_json::json!({
        "models": endpoint.model_catalog_models,
        "current_model_id": endpoint_current_model_id(provider_id, endpoint),
        "meta": {
            "source_kind": "endpoint",
            "catalog_status": endpoint.model_catalog_status,
            "catalog_source": endpoint.model_catalog_source,
            "fetched_at": endpoint.model_catalog_fetched_at,
            "last_error": endpoint.model_catalog_error,
            "stale": stale,
        },
    })
}

fn endpoint_selection_is_active(config: &harness_sources::HarnessProviderSourceConfig) -> bool {
    if config.selected_source_kind != HarnessSourceKind::Endpoint {
        return false;
    }
    let Some(selected_endpoint_id) = config.selected_endpoint_id.as_deref() else {
        return false;
    };
    config
        .endpoints
        .iter()
        .any(|endpoint| endpoint.id == selected_endpoint_id)
}

async fn provider_has_active_auth_config(
    data_root: &std::path::Path,
    provider_id: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> bool {
    if let Some(config) = source_config {
        if endpoint_selection_is_active(config) {
            return true;
        }
        if provider_id == "codex" && config.selected_source_kind == HarnessSourceKind::Subscription
        {
            // Codex can operate in unmanaged host-login mode, where selecting
            // subscription is the strongest configured-auth signal we can rely on.
            return true;
        }
    }
    match crate::provider_accounts::subscription_env_for_active_account(data_root, provider_id)
        .await
    {
        Ok(env) => !env.is_empty(),
        Err(_) => false,
    }
}

fn provider_auth_mode(
    has_active_auth: bool,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> &'static str {
    if !has_active_auth {
        return "none";
    }
    if let Some(config) = source_config {
        if endpoint_selection_is_active(config) {
            return "endpoint";
        }
    }
    "subscription"
}

pub(super) async fn get_provider_options(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);
    const VERIFY_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

    if provider_id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }

    let ws_id = parse_workspace_id(&ws_id)?;

    let cache_key = format!("{}/{}", ws_id.0, provider_id);
    let verify_entry: Option<(std::time::Instant, serde_json::Value)> = state
        .providers
        .verify_cache
        .lock()
        .await
        .get(&cache_key)
        .map(|c| (c.cached_at, c.value.clone()));
    let cached_entry: Option<(std::time::Instant, serde_json::Value)> = state
        .providers
        .options_cache
        .lock()
        .await
        .get(&cache_key)
        .map(|c| (c.cached_at, c.value.clone()));
    if let Some((cached_at, cached_value)) = cached_entry.as_ref() {
        if cached_at.elapsed() < CACHE_TTL {
            let mut out = cached_value.clone();
            if let Some((verify_at, verify)) = verify_entry.as_ref() {
                if verify_at.elapsed() < VERIFY_TTL {
                    if let Some(obj) = out.as_object_mut() {
                        obj.insert("verify".to_string(), verify.clone());
                    }
                }
            }
            return Ok(Json(out));
        }
    }
    let cached_models = cached_entry
        .as_ref()
        .and_then(|(_, value)| value.get("models"))
        .cloned()
        .filter(|v| !v.is_null());
    let cached_modes = cached_entry
        .as_ref()
        .and_then(|(_, value)| value.get("modes"))
        .cloned()
        .filter(|v| !v.is_null());

    let provider_status = state
        .providers
        .statuses
        .lock()
        .await
        .get(&provider_id)
        .cloned();
    let source_config =
        harness_sources::get_provider_source_config(&state.core.data_root, &provider_id)
            .await
            .ok();
    let has_active_auth = provider_has_active_auth_config(
        &state.core.data_root,
        &provider_id,
        source_config.as_ref(),
    )
    .await;
    let auth_mode = provider_auth_mode(has_active_auth, source_config.as_ref());
    let selected_endpoint = selected_endpoint_record_from_harness_config(source_config.as_ref());

    if provider_status.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unsupported provider id: {provider_id}"),
            })),
        ));
    }

    if let Some(st) = provider_status.as_ref() {
        if !st.installed || !matches!(st.health, ctx_providers::adapters::ProviderHealth::Ok) {
            let mut raw_base_resp = serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id.0,
                "installed": st.installed,
                "health": st.health,
                "diagnostics": st.diagnostics,
                "probe_ok": false,
                "probe_error": "provider not installed or unhealthy",
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "probed_at": chrono::Utc::now().to_rfc3339(),
            });
            if let Some(source) = source_config.as_ref() {
                raw_base_resp["source"] =
                    serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
            }
            let base_resp = redact_json_value(raw_base_resp);
            state.providers.options_cache.lock().await.insert(
                cache_key,
                crate::daemon::CachedProviderOptions {
                    cached_at: std::time::Instant::now(),
                    value: base_resp.clone(),
                },
            );
            let mut out = base_resp;
            if let Some((verify_at, verify)) = verify_entry.as_ref() {
                if verify_at.elapsed() < VERIFY_TTL {
                    if let Some(obj) = out.as_object_mut() {
                        obj.insert("verify".to_string(), verify.clone());
                    }
                }
            }
            return Ok(Json(out));
        }
    }

    let use_crp_probe = provider_id == "codex" || provider_id == "claude-crp";
    if !use_crp_probe {
        let now = chrono::Utc::now();
        let mut raw_resp = serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": ws_id.0,
            "installed": provider_status.as_ref().map(|s| s.installed).unwrap_or(true),
            "probe_ok": true,
            "supports_load": false,
            "auth_required": false,
            "has_active_auth": has_active_auth,
            "auth_mode": auth_mode,
            "probed_at": now.to_rfc3339(),
        });
        if let Some(source) = source_config.as_ref() {
            raw_resp["source"] = serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
        }
        if let Some(endpoint) = selected_endpoint.as_ref() {
            raw_resp["models"] = endpoint_models_payload(&provider_id, endpoint, now);
            if harness_sources::endpoint_model_catalog_is_stale(endpoint, now) {
                let state = Arc::clone(&state);
                let provider_id_for_refresh = provider_id.clone();
                let endpoint_id_for_refresh = endpoint.id.clone();
                tokio::spawn(async move {
                    let _ = harness_sources::refresh_provider_endpoint_model_catalog(
                        &state.core.data_root,
                        &provider_id_for_refresh,
                        &endpoint_id_for_refresh,
                    )
                    .await;
                });
            }
        }
        if raw_resp.get("models").is_none() || raw_resp.get("models").is_some_and(|v| v.is_null()) {
            if let Some(models) = cached_models {
                raw_resp["models"] = models;
            }
        }
        if raw_resp.get("modes").is_none() || raw_resp.get("modes").is_some_and(|v| v.is_null()) {
            if let Some(modes) = cached_modes {
                raw_resp["modes"] = modes;
            }
        }

        let resp = redact_json_value(raw_resp);
        state.providers.options_cache.lock().await.insert(
            cache_key,
            crate::daemon::CachedProviderOptions {
                cached_at: std::time::Instant::now(),
                value: resp.clone(),
            },
        );

        let mut out = resp;
        if let Some((verify_at, verify)) = verify_entry.as_ref() {
            if verify_at.elapsed() < VERIFY_TTL {
                if let Some(obj) = out.as_object_mut() {
                    obj.insert("verify".to_string(), verify.clone());
                }
            }
        }
        return Ok(Json(out));
    }

    if use_crp_probe {
        let ws = state
            .global_store()
            .get_workspace(ws_id)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": "failed to load workspace",
                    })),
                )
            })?
            .ok_or((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "workspace not found",
                })),
            ))?;

        if let Some(endpoint) = selected_endpoint.as_ref() {
            let now = chrono::Utc::now();
            let mut raw_resp = serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id.0,
                "installed": provider_status.as_ref().map(|s| s.installed).unwrap_or(true),
                "probe_ok": true,
                "supports_load": false,
                "auth_required": false,
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "models": endpoint_models_payload(&provider_id, endpoint, now),
                "probed_at": now.to_rfc3339(),
            });
            if let Some(source) = source_config.as_ref() {
                raw_resp["source"] =
                    serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
            }
            if harness_sources::endpoint_model_catalog_is_stale(endpoint, now) {
                let state = Arc::clone(&state);
                let provider_id_for_refresh = provider_id.clone();
                let endpoint_id_for_refresh = endpoint.id.clone();
                tokio::spawn(async move {
                    let _ = harness_sources::refresh_provider_endpoint_model_catalog(
                        &state.core.data_root,
                        &provider_id_for_refresh,
                        &endpoint_id_for_refresh,
                    )
                    .await;
                });
            }
            let resp = redact_json_value(raw_resp);
            state.providers.options_cache.lock().await.insert(
                cache_key,
                crate::daemon::CachedProviderOptions {
                    cached_at: std::time::Instant::now(),
                    value: resp.clone(),
                },
            );
            let mut out = resp;
            if let Some((verify_at, verify)) = verify_entry.as_ref() {
                if verify_at.elapsed() < VERIFY_TTL {
                    if let Some(obj) = out.as_object_mut() {
                        obj.insert("verify".to_string(), verify.clone());
                    }
                }
            }
            return Ok(Json(out));
        }

        let cfg = installer::load_agent_server_config(&state.core.data_root)
            .await
            .unwrap_or_default();
        let runtime_command = installer::resolve_runtime_provider_command(&cfg, &provider_id)
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!(
                            "runtime_command_invalid: provider={provider_id} error={e}"
                        ),
                    })),
                )
            })?
            .ok_or((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!(
                        "runtime_command_missing: provider={provider_id} (configure an absolute runtime command)"
                    ),
                })),
            ))?;
        let normalized_runtime = normalize_acp_provider_command(
            &state.core.data_root,
            &provider_id,
            installer::AgentServerCommand {
                command: runtime_command.command_abs_path,
                args: runtime_command.args,
                dependencies: runtime_command.dependencies,
                managed: None,
            },
        );
        let command = normalized_runtime.command;
        let args = normalized_runtime.args;

        let probe = match provider_probe::provider_probe_env_for_workspace_runtime(
            &state,
            &ws,
            &provider_id,
        )
        .await
        {
            Ok((_source, mut env)) => {
                installer::prepend_runtime_bin_dirs_to_provider_path(
                    &mut env,
                    &cfg,
                    &provider_id,
                    &state.core.data_root,
                );
                probe_crp_models(
                    &provider_id,
                    command,
                    args,
                    PathBuf::from(&ws.root_path),
                    env,
                )
                .await
            }
            Err(err) => Err(anyhow::anyhow!(err)),
        };

        let mut raw_resp = match probe {
            Ok(probe) => serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id.0,
                "installed": provider_status.as_ref().map(|s| s.installed).unwrap_or(true),
                "probe_ok": true,
                "supports_load": false,
                "auth_required": false,
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "models": {
                    "models": probe.models,
                    "current_model_id": probe.current_model_id,
                },
                "probed_at": chrono::Utc::now().to_rfc3339(),
            }),
            Err(e) => serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id.0,
                "installed": provider_status.as_ref().map(|s| s.installed).unwrap_or(false),
                "probe_ok": false,
                "probe_error": logs::redact_sensitive(&e.to_string()),
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "probed_at": chrono::Utc::now().to_rfc3339(),
            }),
        };
        if let Some(source) = source_config.as_ref() {
            raw_resp["source"] = serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
        }

        if raw_resp.get("models").is_none() || raw_resp.get("models").is_some_and(|v| v.is_null()) {
            if let Some(models) = cached_models {
                raw_resp["models"] = models;
            }
        }
        if raw_resp.get("modes").is_none() || raw_resp.get("modes").is_some_and(|v| v.is_null()) {
            if let Some(modes) = cached_modes {
                raw_resp["modes"] = modes;
            }
        }

        let resp = redact_json_value(raw_resp);
        state.providers.options_cache.lock().await.insert(
            cache_key,
            crate::daemon::CachedProviderOptions {
                cached_at: std::time::Instant::now(),
                value: resp.clone(),
            },
        );

        let mut out = resp;
        if let Some((verify_at, verify)) = verify_entry.as_ref() {
            if verify_at.elapsed() < VERIFY_TTL {
                if let Some(obj) = out.as_object_mut() {
                    obj.insert("verify".to_string(), verify.clone());
                }
            }
        }
        return Ok(Json(out));
    }

    Err((
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": "unsupported provider id",
        })),
    ))
}

pub(super) async fn get_provider_harness_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    if id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let config = harness_sources::get_provider_source_config(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": logs::redact_sensitive(&e.to_string()),
                })),
            )
        })?;
    Ok(Json(config))
}

pub(super) async fn select_provider_harness_source(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SelectHarnessSourceReq>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    if id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let config = harness_sources::set_provider_source_selection(
        &state.core.data_root,
        &id,
        req.source_kind,
        req.endpoint_id,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": logs::redact_sensitive(&e.to_string()),
            })),
        )
    })?;
    invalidate_provider_probe_caches(&state, &id).await;
    Ok(Json(config))
}

pub(super) async fn upsert_provider_harness_endpoint(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpsertHarnessEndpointReq>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    if id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let endpoint = harness_sources::upsert_provider_endpoint(
        &state.core.data_root,
        &id,
        HarnessEndpointUpsert {
            endpoint_id: req.endpoint_id,
            name: req.name,
            base_url: req.base_url,
            api_shape: req.api_shape,
            auth_type: req.auth_type,
            model_override: req.model_override,
            api_key: req.api_key,
        },
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": logs::redact_sensitive(&e.to_string()),
            })),
        )
    })?;
    if let Some(manual_model_ids) = req.manual_model_ids {
        let _ = harness_sources::set_provider_endpoint_manual_models(
            &state.core.data_root,
            &id,
            &endpoint.id,
            manual_model_ids,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": logs::redact_sensitive(&e.to_string()),
                })),
            )
        })?;
    }
    let _ = harness_sources::refresh_provider_endpoint_model_catalog(
        &state.core.data_root,
        &id,
        &endpoint.id,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": logs::redact_sensitive(&e.to_string()),
            })),
        )
    })?;
    invalidate_provider_probe_caches(&state, &id).await;
    let config = harness_sources::get_provider_source_config(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": logs::redact_sensitive(&e.to_string()),
                })),
            )
        })?;
    Ok(Json(config))
}

pub(super) async fn refresh_provider_harness_endpoint_models(
    State(state): State<Arc<AppState>>,
    Path((id, endpoint_id)): Path<(String, String)>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    if id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    harness_sources::refresh_provider_endpoint_model_catalog(
        &state.core.data_root,
        &id,
        &endpoint_id,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": logs::redact_sensitive(&e.to_string()),
            })),
        )
    })?;
    invalidate_provider_probe_caches(&state, &id).await;
    let config = harness_sources::get_provider_source_config(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": logs::redact_sensitive(&e.to_string()),
                })),
            )
        })?;
    Ok(Json(config))
}

pub(super) async fn set_provider_harness_endpoint_manual_models(
    State(state): State<Arc<AppState>>,
    Path((id, endpoint_id)): Path<(String, String)>,
    Json(req): Json<SetEndpointManualModelsReq>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    if id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    harness_sources::set_provider_endpoint_manual_models(
        &state.core.data_root,
        &id,
        &endpoint_id,
        req.model_ids,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": logs::redact_sensitive(&e.to_string()),
            })),
        )
    })?;
    invalidate_provider_probe_caches(&state, &id).await;
    let config = harness_sources::get_provider_source_config(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": logs::redact_sensitive(&e.to_string()),
                })),
            )
        })?;
    Ok(Json(config))
}

pub(super) async fn delete_provider_harness_endpoint(
    State(state): State<Arc<AppState>>,
    Path((id, endpoint_id)): Path<(String, String)>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    if id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let config =
        harness_sources::delete_provider_endpoint(&state.core.data_root, &id, &endpoint_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": logs::redact_sensitive(&e.to_string()),
                    })),
                )
            })?;
    invalidate_provider_probe_caches(&state, &id).await;
    Ok(Json(config))
}

pub(super) async fn verify_provider_for_workspace(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<ProviderAuthCheckResp>, (StatusCode, Json<serde_json::Value>)> {
    if provider_id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let ws_id = parse_workspace_id(&ws_id)?;

    let workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "failed to load workspace",
                })),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "workspace not found",
            })),
        ))?;

    let provider_status = state
        .providers
        .statuses
        .lock()
        .await
        .get(&provider_id)
        .cloned()
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unsupported provider id: {provider_id}"),
            })),
        ))?;

    let checked_at = Utc::now().to_rfc3339();
    let mut status = "ok".to_string();
    let mut auth_required = Some(false);
    let mut message: Option<String> = None;
    let mut endpoint_status = HarnessEndpointVerificationStatus::Valid;
    let mut selected_endpoint_id: Option<String> = selected_endpoint_from_harness_config(
        harness_sources::get_provider_source_config(&state.core.data_root, &provider_id)
            .await
            .ok(),
    );

    if !provider_status.installed
        || !matches!(
            provider_status.health,
            ctx_providers::adapters::ProviderHealth::Ok
        )
    {
        status = "error".to_string();
        auth_required = Some(false);
        message = Some("provider not installed or unhealthy".to_string());
        endpoint_status = HarnessEndpointVerificationStatus::Error;
    } else {
        let cfg = installer::load_agent_server_config(&state.core.data_root)
            .await
            .unwrap_or_default();
        let runtime_command = installer::resolve_runtime_provider_command(&cfg, &provider_id)
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!(
                            "runtime_command_invalid: provider={provider_id} error={e}"
                        ),
                    })),
                )
            })?
            .ok_or((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!(
                        "runtime_command_missing: provider={provider_id} (configure an absolute runtime command)"
                    ),
                })),
            ))?;
        let normalized_runtime = normalize_acp_provider_command(
            &state.core.data_root,
            &provider_id,
            installer::AgentServerCommand {
                command: runtime_command.command_abs_path,
                args: runtime_command.args,
                dependencies: runtime_command.dependencies,
                managed: None,
            },
        );
        let command = normalized_runtime.command;
        let args = normalized_runtime.args;

        match provider_probe::provider_probe_env_for_workspace_runtime(
            &state,
            &workspace,
            &provider_id,
        )
        .await
        {
            Ok((source, mut env)) => {
                installer::prepend_runtime_bin_dirs_to_provider_path(
                    &mut env,
                    &cfg,
                    &provider_id,
                    &state.core.data_root,
                );
                if source.source_kind == HarnessSourceKind::Endpoint {
                    selected_endpoint_id = source
                        .endpoint
                        .as_ref()
                        .map(|ep| ep.id.clone())
                        .or(selected_endpoint_id);
                } else {
                    selected_endpoint_id = None;
                }
                let probe = probe_crp_models(
                    &provider_id,
                    command,
                    args,
                    PathBuf::from(&workspace.root_path),
                    env,
                )
                .await;
                if let Err(err) = probe {
                    let msg = logs::redact_sensitive(&err.to_string());
                    let (classified, auth, endpoint_verify) = classify_probe_error(&msg);
                    status = classified.to_string();
                    auth_required = auth;
                    message = Some(msg);
                    endpoint_status = endpoint_verify;
                }
            }
            Err(err) => {
                let msg = logs::redact_sensitive(&err);
                let (classified, auth, endpoint_verify) = classify_probe_error(&msg);
                status = classified.to_string();
                auth_required = auth;
                message = Some(msg);
                endpoint_status = endpoint_verify;
            }
        }
    }

    if let Some(endpoint_id) = selected_endpoint_id.as_ref() {
        let _ = harness_sources::mark_endpoint_verification(
            &state.core.data_root,
            &provider_id,
            endpoint_id,
            endpoint_status,
            message.clone(),
        )
        .await;
    }

    let resp = ProviderAuthCheckResp {
        provider_id: provider_id.clone(),
        workspace_id: ws_id.0.to_string(),
        status: status.clone(),
        auth_required,
        checked_at: Some(checked_at),
        message: message.clone(),
    };

    let verify_value =
        redact_json_value(serde_json::to_value(&resp).unwrap_or(serde_json::Value::Null));
    let cache_key = format!("{}/{}", ws_id.0, provider_id);
    state.providers.verify_cache.lock().await.insert(
        cache_key,
        crate::daemon::CachedProviderVerify {
            cached_at: std::time::Instant::now(),
            value: verify_value,
        },
    );

    Ok(Json(resp))
}

pub(super) async fn authenticate_provider_for_workspace(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
    req: Option<Json<AuthenticateProviderReq>>,
) -> Result<Json<ProviderAuthCheckResp>, (StatusCode, Json<serde_json::Value>)> {
    if provider_id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let ws_id = parse_workspace_id(&ws_id)?;
    let method_id = req.and_then(|value| value.0.method_id);

    let workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "failed to load workspace",
                })),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "workspace not found",
            })),
        ))?;

    let (source, provider_env) =
        provider_probe::provider_probe_env_for_workspace_runtime(&state, &workspace, &provider_id)
            .await
            .map_err(|err| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": err,
                    })),
                )
            })?;
    if source.source_kind == HarnessSourceKind::Endpoint {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "selected source is endpoint; update endpoint key/config directly instead of interactive authenticate",
            })),
        ));
    }

    let adapter = {
        let map = state.providers.adapters.lock().await;
        map.get(&provider_id).cloned()
    }
    .ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": "provider adapter not available",
        })),
    ))?;

    let (event_tx, mut event_rx) = mpsc::channel(32);
    tokio::spawn(async move { while event_rx.recv().await.is_some() {} });

    let checked_at = Utc::now().to_rfc3339();
    let result = adapter
        .authenticate_session(
            format!("auth-{}", uuid::Uuid::new_v4()),
            PathBuf::from(workspace.root_path),
            provider_env,
            method_id,
            event_tx,
        )
        .await;

    let resp = match result {
        Ok(()) => ProviderAuthCheckResp {
            provider_id: provider_id.clone(),
            workspace_id: ws_id.0.to_string(),
            status: "ok".to_string(),
            auth_required: Some(false),
            checked_at: Some(checked_at),
            message: None,
        },
        Err(err) => {
            let msg = logs::redact_sensitive(&err.to_string());
            let (status, auth_required, _) = classify_probe_error(&msg);
            ProviderAuthCheckResp {
                provider_id: provider_id.clone(),
                workspace_id: ws_id.0.to_string(),
                status: status.to_string(),
                auth_required,
                checked_at: Some(checked_at),
                message: Some(msg),
            }
        }
    };

    let verify_value =
        redact_json_value(serde_json::to_value(&resp).unwrap_or(serde_json::Value::Null));
    let cache_key = format!("{}/{}", ws_id.0, provider_id);
    state.providers.verify_cache.lock().await.insert(
        cache_key,
        crate::daemon::CachedProviderVerify {
            cached_at: std::time::Instant::now(),
            value: verify_value,
        },
    );

    Ok(Json(resp))
}

pub(super) async fn install_provider(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<InstallStartResponse>, (StatusCode, Json<serde_json::Value>)> {
    if id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let target = installer::parse_install_target(query.target.as_deref()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": e.to_string()
            })),
        )
    })?;

    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    if !installer::is_supported_managed_provider_for_target(&matrix, &id, target) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!(
                    "unsupported provider for managed install target '{}': {id}",
                    target.as_str()
                )
            })),
        ));
    }

    let (install_id, started_new) = state.start_install(id.clone(), Some(target)).await;
    if started_new {
        let state2 = state.clone();
        let provider_id = id.clone();
        tokio::spawn(async move {
            if let Err(e) = installer::install_provider_with_progress(
                state2.clone(),
                install_id,
                provider_id.clone(),
                target,
            )
            .await
            {
                tracing::error!("provider install failed ({provider_id}): {e:#}");
            }
        });
    }

    Ok(Json(InstallStartResponse {
        provider_id: id,
        install_id,
        target,
    }))
}

#[derive(Debug, Serialize)]
pub(super) struct LspInstallStartResponse {
    server_id: String,
    install_id: InstallId,
}

pub(super) async fn install_lsp_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<LspInstallStartResponse>, StatusCode> {
    if !installer::is_supported_managed_lsp_server(&id) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let install_key = format!("lsp:{id}");
    let (install_id, started_new) = state.start_install(install_key, None).await;
    if started_new {
        let state2 = state.clone();
        let server_id = id.clone();
        tokio::spawn(async move {
            if let Err(e) = installer::install_lsp_server_with_progress(
                state2.clone(),
                install_id,
                server_id.clone(),
            )
            .await
            {
                tracing::error!("lsp install failed ({server_id}): {e:#}");
            }
        });
    }

    Ok(Json(LspInstallStartResponse {
        server_id: id,
        install_id,
    }))
}

pub(super) async fn install_all_providers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<Vec<InstallStartResponse>>, StatusCode> {
    let target = installer::parse_install_target(query.target.as_deref())
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut out = Vec::new();
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    for entry in &matrix.providers {
        if !installer::is_supported_managed_provider_for_target(&matrix, &entry.id, target) {
            continue;
        }
        let id = entry.id.as_str();
        if let Some(install_id) = state.find_running_install(id, Some(target)).await {
            out.push(InstallStartResponse {
                provider_id: id.to_string(),
                install_id,
                target,
            });
            continue;
        }

        let status = state.providers.statuses.lock().await.get(id).cloned();
        if let Some(st) = status {
            if should_skip_install_for_healthy_provider(&st) {
                continue;
            }
        }

        let (install_id, started_new) = state.start_install(id.to_string(), Some(target)).await;
        if started_new {
            let state2 = state.clone();
            let provider_id = id.to_string();
            tokio::spawn(async move {
                if let Err(e) = installer::install_provider_with_progress(
                    state2.clone(),
                    install_id,
                    provider_id.clone(),
                    target,
                )
                .await
                {
                    tracing::error!("provider install failed ({provider_id}): {e:#}");
                }
            });
        }
        out.push(InstallStartResponse {
            provider_id: id.to_string(),
            install_id,
            target,
        });
    }
    Ok(Json(out))
}

fn has_provider_update_available(status: &ctx_providers::adapters::ProviderStatus) -> bool {
    let matrix_update = status
        .details
        .get("matrix_update_available")
        .map(|value| value == "true")
        .unwrap_or(false);
    let dependency_update = status
        .details
        .get("managed_dependency_update_available")
        .map(|value| value == "true")
        .unwrap_or(false);
    matrix_update || dependency_update
}

fn should_skip_install_for_healthy_provider(
    status: &ctx_providers::adapters::ProviderStatus,
) -> bool {
    status.installed
        && matches!(status.health, ctx_providers::adapters::ProviderHealth::Ok)
        && !has_provider_update_available(status)
}

#[derive(Debug, Serialize)]
pub(super) struct MatrixRefreshResponse {
    provider_count: usize,
    generated_at: Option<String>,
}

pub(super) async fn refresh_provider_matrix(
    State(state): State<Arc<AppState>>,
) -> Result<Json<MatrixRefreshResponse>, (StatusCode, Json<ApiErrorResp>)> {
    crate::provider_matrix::invalidate_matrix_cache(&state.providers.matrix_cache).await;
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    installer::refresh_provider_statuses(state.as_ref())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: format!("failed to refresh provider statuses: {e:#}"),
                }),
            )
        })?;
    Ok(Json(MatrixRefreshResponse {
        provider_count: matrix.providers.len(),
        generated_at: matrix.generated_at,
    }))
}

pub(super) async fn get_install(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Json<InstallInfo>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .get_install_info(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub(super) async fn cancel_install(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Json<InstallInfo>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .cancel_install(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub(super) async fn list_install_events(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Json<Vec<InstallProgressEvent>>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .get_install_events(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub(super) async fn install_stream_sse(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, axum::Error>>>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let Some(sender) = state.get_install_sender(install_id).await else {
        return Err(StatusCode::NOT_FOUND);
    };

    let history = state
        .get_install_events(install_id)
        .await
        .unwrap_or_default();
    let initial = futures::stream::iter(history.into_iter().map(|ev| {
        let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
        Ok::<_, axum::Error>(SseEvent::default().event("progress").data(payload))
    }));

    let live = futures::stream::unfold(sender.subscribe(), move |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                    return Some((Ok(SseEvent::default().event("progress").data(payload)), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    let stream = initial.chain(live);

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15))))
}

#[derive(Debug, Deserialize)]
pub(super) struct DevRestartProvidersReq {
    mode: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DevRestartProvidersResp {
    mode: String,
    results: Vec<DevRestartProvidersResult>,
}

#[derive(Debug, Serialize)]
pub(super) struct DevRestartProvidersResult {
    provider_id: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

fn dev_tools_enabled() -> bool {
    std::env::var("CTX_DEV_MODE")
        .ok()
        .map(|raw| {
            let v = raw.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

fn parse_restart_mode(value: &str) -> Option<ProviderRestartMode> {
    match value.trim().to_lowercase().as_str() {
        "immediate" => Some(ProviderRestartMode::Immediate),
        "drain" => Some(ProviderRestartMode::Drain),
        _ => None,
    }
}

pub(super) async fn dev_restart_providers(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DevRestartProvidersReq>,
) -> Result<Json<DevRestartProvidersResp>, (StatusCode, Json<ApiErrorResp>)> {
    if !dev_tools_enabled() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "dev tools are disabled".to_string(),
            }),
        ));
    }

    let Some(mode) = parse_restart_mode(&req.mode) else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "mode must be 'immediate' or 'drain'".to_string(),
            }),
        ));
    };
    let reason = req
        .reason
        .unwrap_or_else(|| format!("dev restart ({})", mode.as_str()));

    let adapters = {
        let providers = state.providers.adapters.lock().await;
        providers
            .iter()
            .map(|(id, adapter)| (id.clone(), Arc::clone(adapter)))
            .collect::<Vec<_>>()
    };

    let mut results = Vec::with_capacity(adapters.len());
    for (provider_id, adapter) in adapters {
        match adapter.restart(&reason, mode).await {
            Ok(()) => results.push(DevRestartProvidersResult {
                provider_id,
                status: "ok".to_string(),
                message: None,
            }),
            Err(err) => {
                let message = err.to_string();
                let status = if message.to_lowercase().contains("does not support") {
                    "unsupported"
                } else {
                    "error"
                };
                results.push(DevRestartProvidersResult {
                    provider_id,
                    status: status.to_string(),
                    message: Some(message),
                });
            }
        }
    }

    Ok(Json(DevRestartProvidersResp {
        mode: mode.as_str().to_string(),
        results,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_endpoint(id: &str) -> harness_sources::HarnessEndpointRecord {
        harness_sources::HarnessEndpointRecord {
            id: id.to_string(),
            provider_id: "codex".to_string(),
            name: "Test endpoint".to_string(),
            base_url: Some("https://api.openai.com/v1".to_string()),
            api_shape: HarnessApiShape::OpenaiResponses,
            auth_type: "bearer".to_string(),
            model_override: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_verification_status: harness_sources::HarnessEndpointVerificationStatus::Unknown,
            last_verification_at: None,
            last_error: None,
            has_api_key: true,
            model_catalog_status: harness_sources::EndpointModelCatalogStatus::Unknown,
            model_catalog_fetched_at: None,
            model_catalog_error: None,
            model_catalog_models: Vec::new(),
            manual_model_ids: Vec::new(),
            model_catalog_source: None,
        }
    }

    #[test]
    fn expected_callback_extracts_loopback_redirect() {
        let auth_url = "https://chat.openai.com/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A6543%2Fauth%2Fcallback";
        let expected = expected_callback_from_auth_url(auth_url);
        assert_eq!(
            expected.as_deref(),
            Some("http://localhost:6543/auth/callback")
        );
    }

    #[test]
    fn callback_validation_rejects_non_loopback_host() {
        let err = validate_callback_url(
            "http://example.com:1234/auth/callback?code=abc",
            Some("http://localhost:1234/auth/callback"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("loopback"));
    }

    #[test]
    fn callback_validation_accepts_expected_port_path_and_query() {
        validate_callback_url(
            "http://127.0.0.1:4321/auth/callback?code=abc&state=def",
            Some("http://localhost:4321/auth/callback"),
        )
        .expect("callback URL should validate");
    }

    #[test]
    fn extract_auth_url_detects_urls_in_line() {
        let line = "Open this URL to continue: https://claude.ai/oauth/authorize?foo=bar";
        assert_eq!(
            extract_auth_url(line).as_deref(),
            Some("https://claude.ai/oauth/authorize?foo=bar")
        );
    }

    #[test]
    fn extract_auth_url_reconstructs_wrapped_url_lines() {
        let wrapped =
            "Open this URL: https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A\n64111%2Fauth%2Fcallback&state=abc";
        assert_eq!(
            extract_auth_url(wrapped).as_deref(),
            Some(
                "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A64111%2Fauth%2Fcallback&state=abc"
            )
        );
    }

    #[test]
    fn extract_auth_url_from_value_detects_embedded_url_in_message() {
        let payload = serde_json::json!({
            "message": "Visit this link to sign in: https://accounts.google.com/o/oauth2/auth?foo=bar"
        });
        assert_eq!(
            extract_auth_url_from_value(&payload).as_deref(),
            Some("https://accounts.google.com/o/oauth2/auth?foo=bar")
        );
    }

    #[test]
    fn normalize_claude_login_line_strips_ansi_sequences() {
        let raw = "\u{1b}[90mOpen URL:\u{1b}[0m https://claude.ai/oauth/authorize?foo=bar\r";
        let normalized = normalize_claude_login_line(raw);
        assert_eq!(
            extract_auth_url(&normalized).as_deref(),
            Some("https://claude.ai/oauth/authorize?foo=bar")
        );
    }

    #[test]
    fn normalize_claude_login_line_extracts_url_from_osc8_sequence() {
        let raw =
            "\u{1b}]8;;https://claude.ai/oauth/authorize?foo=bar\u{7}Sign in\u{1b}]8;;\u{7}\r";
        let normalized = normalize_claude_login_line(raw);
        assert_eq!(
            extract_auth_url(&normalized).as_deref(),
            Some("https://claude.ai/oauth/authorize?foo=bar")
        );
    }

    #[test]
    fn auth_url_looks_complete_accepts_hosted_redirect_callback() {
        let url = "https://claude.ai/oauth/authorize?redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback";
        assert!(auth_url_looks_complete(url));
    }

    #[test]
    fn auth_url_looks_complete_requires_port_for_loopback_callback() {
        let url =
            "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%2Fcallback";
        assert!(!auth_url_looks_complete(url));
        let with_port =
            "https://claude.ai/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A5999%2Fcallback";
        assert!(auth_url_looks_complete(with_port));
    }

    #[tokio::test]
    async fn read_trailing_claude_login_lines_waits_for_late_arrival() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            let _ = tx.send("sk-ant-oat01-late-token".to_string());
        });
        let lines = read_trailing_claude_login_lines(&mut rx, Duration::from_millis(120)).await;
        assert_eq!(lines, vec!["sk-ant-oat01-late-token".to_string()]);
    }

    #[test]
    fn should_attempt_claude_cli_bootstrap_for_runtime_command_resolution_failures() {
        let ok_missing: anyhow::Result<Option<installer::ProviderRuntimeCommand>> = Ok(None);
        assert!(should_attempt_claude_cli_bootstrap(&ok_missing));

        let ok_present: anyhow::Result<Option<installer::ProviderRuntimeCommand>> =
            Ok(Some(installer::ProviderRuntimeCommand {
                provider_id: "claude-cli".to_string(),
                command_abs_path: "/tmp/claude".to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                source: installer::ProviderRuntimeCommandSource::UserOverride,
            }));
        assert!(!should_attempt_claude_cli_bootstrap(&ok_present));

        let missing_err: anyhow::Result<Option<installer::ProviderRuntimeCommand>> =
            Err(anyhow::anyhow!(
                "runtime_command_not_found: provider=claude-cli source=managed_install command=/tmp/missing"
            ));
        assert!(should_attempt_claude_cli_bootstrap(&missing_err));

        let unrelated_err: anyhow::Result<Option<installer::ProviderRuntimeCommand>> =
            Err(anyhow::anyhow!("network timeout"));
        assert!(!should_attempt_claude_cli_bootstrap(&unrelated_err));
    }

    #[tokio::test]
    async fn resolve_claude_setup_token_runtime_bootstraps_missing_runtime_command() {
        let temp = tempfile::tempdir().expect("tempdir");
        let data_root = temp.path().to_path_buf();
        let runtime_path = data_root.join("claude-cli-mock.sh");
        std::fs::write(&runtime_path, "#!/bin/sh\nexit 0\n").expect("write runtime");
        let runtime_path_str = runtime_path.to_string_lossy().to_string();
        let install_called = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let resolved = resolve_claude_setup_token_runtime_with_bootstrap(&data_root, || {
            let data_root = data_root.clone();
            let runtime_path_str = runtime_path_str.clone();
            let install_called = Arc::clone(&install_called);
            async move {
                install_called.store(true, std::sync::atomic::Ordering::SeqCst);
                let mut cfg = installer::load_agent_server_config(&data_root)
                    .await
                    .context("loading config in bootstrap test")?;
                cfg.providers.insert(
                    "claude-cli".to_string(),
                    installer::AgentServerCommand {
                        command: runtime_path_str.clone(),
                        args: vec!["--shim".to_string()],
                        dependencies: Vec::new(),
                        managed: None,
                    },
                );
                installer::save_agent_server_config(&data_root, &cfg)
                    .await
                    .context("saving config in bootstrap test")?;
                Ok(())
            }
        })
        .await
        .expect("resolve runtime with bootstrap");

        assert!(install_called.load(std::sync::atomic::Ordering::SeqCst));
        assert!(resolved.command_abs_path.contains("claude-cli-mock.sh"));
        assert_eq!(resolved.args, vec!["--shim".to_string()]);
    }

    #[test]
    fn extract_claude_setup_token_handles_wrapped_output() {
        let output = r#"
Long-lived authentication token created successfully!

Your OAuth token (valid for 1 year):

sk-ant-oat01-1WRAPPED_TEST_ONLY_0123456789_WR
APPED_TEST_ONLY_0123456789_WRAPPED_TEST_ONLY_
0123456789_WRAPPED

Store this token securely.
"#;
        let token = extract_claude_setup_token(output).expect("token should parse");
        assert!(token.starts_with("sk-ant-oat01-"));
        assert!(token.contains("APPED_TEST_ONLY_0123456789_WRAPPED_TEST_ONLY_"));
        assert!(token.ends_with("0123456789_WRAPPED"));
    }

    #[test]
    fn extract_claude_setup_token_ignores_wrapped_prose_after_token() {
        let output = r#"
Your OAuth token (valid for 1 year):

sk-ant-oat01-1WRAPPED_TEST_ONLY_0123456789_WR
APPED_TEST_ONLY_0123456789_WRAPPED_TEST_ONLY_
0123456789_WRAPPED
Store
this
token
securely
You
won't
be
able
to
see
it
again
"#;
        let token = extract_claude_setup_token(output).expect("token should parse");
        assert!(token.starts_with("sk-ant-oat01-"));
        assert!(token.ends_with("0123456789_WRAPPED"));
        assert!(!token.contains("Store"));
        assert!(!token.contains("securely"));
    }

    #[test]
    fn extract_claude_setup_token_stops_before_inline_prose() {
        let output = "sk-ant-oat01-1INLINE_TEST_ONLY_0123456789_INLINE_TEST_ONLY_0123456789_INLINE_TEST_ONLY_0123456789_INLINE_TESStore this token securely.";
        let token = extract_claude_setup_token(output).expect("token should parse");
        assert!(token.starts_with("sk-ant-oat01-"));
        assert!(token.ends_with("INE_TES"));
        assert!(!token.contains("Store"));
        assert!(!token.contains("securely"));
    }

    #[test]
    fn extract_claude_setup_token_accepts_short_final_fragment() {
        let output = r#"
Your OAuth token (valid for 1 year):

sk-ant-oat01-abcDEF1234567890_
ZXY987654321
"#;
        let token = extract_claude_setup_token(output).expect("token should parse");
        assert_eq!(token, "sk-ant-oat01-abcDEF1234567890_ZXY987654321");
    }

    #[test]
    fn cache_key_provider_matcher_works() {
        assert!(cache_key_matches_provider(
            "7f72430e-4c43-499f-b54d-6ce2deaed4a0/codex",
            "codex"
        ));
        assert!(!cache_key_matches_provider(
            "7f72430e-4c43-499f-b54d-6ce2deaed4a0/claude-crp",
            "codex"
        ));
        assert!(!cache_key_matches_provider("not-a-key", "codex"));
    }

    #[test]
    fn selected_endpoint_from_harness_config_prefers_endpoint_selection() {
        let endpoint = selected_endpoint_from_harness_config(Some(
            harness_sources::HarnessProviderSourceConfig {
                provider_id: "codex".to_string(),
                selected_source_kind: HarnessSourceKind::Endpoint,
                selected_endpoint_id: Some("ep-123".to_string()),
                endpoints: Vec::new(),
            },
        ));
        assert_eq!(endpoint.as_deref(), Some("ep-123"));

        let subscription = selected_endpoint_from_harness_config(Some(
            harness_sources::HarnessProviderSourceConfig {
                provider_id: "codex".to_string(),
                selected_source_kind: HarnessSourceKind::Subscription,
                selected_endpoint_id: Some("ep-123".to_string()),
                endpoints: Vec::new(),
            },
        ));
        assert!(subscription.is_none());
    }

    #[test]
    fn selected_endpoint_record_from_harness_config_returns_selected_record() {
        let selected = selected_endpoint_record_from_harness_config(Some(
            &harness_sources::HarnessProviderSourceConfig {
                provider_id: "codex".to_string(),
                selected_source_kind: HarnessSourceKind::Endpoint,
                selected_endpoint_id: Some("ep-2".to_string()),
                endpoints: vec![test_endpoint("ep-1"), test_endpoint("ep-2")],
            },
        ))
        .expect("selected endpoint");
        assert_eq!(selected.id, "ep-2");

        let missing = selected_endpoint_record_from_harness_config(Some(
            &harness_sources::HarnessProviderSourceConfig {
                provider_id: "codex".to_string(),
                selected_source_kind: HarnessSourceKind::Endpoint,
                selected_endpoint_id: Some("ep-3".to_string()),
                endpoints: vec![test_endpoint("ep-1"), test_endpoint("ep-2")],
            },
        ));
        assert!(missing.is_none());
    }

    #[test]
    fn endpoint_models_payload_includes_models_and_meta() {
        let now = Utc::now();
        let mut endpoint = test_endpoint("ep-1");
        endpoint.model_override = Some("openai/gpt-5.2".to_string());
        endpoint.model_catalog_status = harness_sources::EndpointModelCatalogStatus::Ready;
        endpoint.model_catalog_fetched_at = Some(now);
        endpoint.model_catalog_source = Some("mixed".to_string());
        endpoint.model_catalog_models = vec![harness_sources::EndpointModelRecord {
            id: "openai/gpt-5.2".to_string(),
            name: Some("GPT-5.2".to_string()),
        }];

        let payload = endpoint_models_payload("codex", &endpoint, now);
        assert_eq!(
            payload
                .pointer("/models/0/id")
                .and_then(serde_json::Value::as_str),
            Some("openai/gpt-5.2")
        );
        assert_eq!(
            payload
                .pointer("/current_model_id")
                .and_then(serde_json::Value::as_str),
            Some("openai/gpt-5.2")
        );
        assert_eq!(
            payload
                .pointer("/meta/catalog_status")
                .and_then(serde_json::Value::as_str),
            Some("ready")
        );
        assert_eq!(
            payload
                .pointer("/meta/catalog_source")
                .and_then(serde_json::Value::as_str),
            Some("mixed")
        );
        assert_eq!(
            payload
                .pointer("/meta/source_kind")
                .and_then(serde_json::Value::as_str),
            Some("endpoint")
        );
        assert_eq!(
            payload
                .pointer("/meta/stale")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn endpoint_models_payload_prefixes_droid_model_override_with_custom_namespace() {
        let now = Utc::now();
        let mut endpoint = test_endpoint("ep-1");
        endpoint.model_override = Some("openai/gpt-5.2".to_string());

        let payload = endpoint_models_payload("droid", &endpoint, now);
        assert_eq!(
            payload
                .pointer("/current_model_id")
                .and_then(serde_json::Value::as_str),
            Some("custom:openai/gpt-5.2")
        );
    }

    #[test]
    fn endpoint_selection_is_active_requires_selected_endpoint_record() {
        let active = harness_sources::HarnessProviderSourceConfig {
            provider_id: "codex".to_string(),
            selected_source_kind: HarnessSourceKind::Endpoint,
            selected_endpoint_id: Some("ep-1".to_string()),
            endpoints: vec![test_endpoint("ep-1")],
        };
        assert!(endpoint_selection_is_active(&active));

        let missing = harness_sources::HarnessProviderSourceConfig {
            provider_id: "codex".to_string(),
            selected_source_kind: HarnessSourceKind::Endpoint,
            selected_endpoint_id: Some("ep-2".to_string()),
            endpoints: vec![test_endpoint("ep-1")],
        };
        assert!(!endpoint_selection_is_active(&missing));
    }

    #[test]
    fn provider_auth_mode_prefers_endpoint_for_active_endpoint_selection() {
        let endpoint = harness_sources::HarnessProviderSourceConfig {
            provider_id: "codex".to_string(),
            selected_source_kind: HarnessSourceKind::Endpoint,
            selected_endpoint_id: Some("ep-1".to_string()),
            endpoints: vec![test_endpoint("ep-1")],
        };
        assert_eq!(provider_auth_mode(true, Some(&endpoint)), "endpoint");

        let subscription = harness_sources::HarnessProviderSourceConfig {
            provider_id: "codex".to_string(),
            selected_source_kind: HarnessSourceKind::Subscription,
            selected_endpoint_id: None,
            endpoints: vec![],
        };
        assert_eq!(
            provider_auth_mode(true, Some(&subscription)),
            "subscription"
        );
        assert_eq!(provider_auth_mode(false, Some(&subscription)), "none");
    }

    #[tokio::test]
    async fn cursor_subscription_selection_requires_managed_account() {
        let root = tempfile::tempdir().expect("tempdir");
        let source = harness_sources::HarnessProviderSourceConfig {
            provider_id: "cursor".to_string(),
            selected_source_kind: HarnessSourceKind::Subscription,
            selected_endpoint_id: None,
            endpoints: vec![],
        };
        let active = provider_has_active_auth_config(root.path(), "cursor", Some(&source)).await;
        assert!(!active);
    }

    #[tokio::test]
    async fn codex_subscription_selection_counts_as_active_auth_config() {
        let root = tempfile::tempdir().expect("tempdir");
        let source = harness_sources::HarnessProviderSourceConfig {
            provider_id: "codex".to_string(),
            selected_source_kind: HarnessSourceKind::Subscription,
            selected_endpoint_id: None,
            endpoints: vec![],
        };
        let active = provider_has_active_auth_config(root.path(), "codex", Some(&source)).await;
        assert!(active);
    }

    #[tokio::test]
    async fn amp_subscription_selection_requires_managed_account() {
        let root = tempfile::tempdir().expect("tempdir");
        let source = harness_sources::HarnessProviderSourceConfig {
            provider_id: "amp".to_string(),
            selected_source_kind: HarnessSourceKind::Subscription,
            selected_endpoint_id: None,
            endpoints: vec![],
        };
        let active = provider_has_active_auth_config(root.path(), "amp", Some(&source)).await;
        assert!(!active);
    }

    #[tokio::test]
    async fn amp_active_account_counts_as_active_auth_config() {
        let root = tempfile::tempdir().expect("tempdir");
        provider_accounts::upsert_amp_account(
            root.path(),
            Some("Amp Test".to_string()),
            Some("amp@example.com".to_string()),
        )
        .await
        .expect("upsert amp account");
        let source = harness_sources::HarnessProviderSourceConfig {
            provider_id: "amp".to_string(),
            selected_source_kind: HarnessSourceKind::Subscription,
            selected_endpoint_id: None,
            endpoints: vec![],
        };
        let active = provider_has_active_auth_config(root.path(), "amp", Some(&source)).await;
        assert!(active);
    }

    #[test]
    fn import_result_restart_filter_treats_already_imported_as_mutation() {
        let already_imported = provider_auth_import::ProviderAuthImportResult {
            candidate_id: "cand-1".to_string(),
            provider_id: "claude-crp".to_string(),
            status: "already_imported".to_string(),
            profile_id: Some("acct-1".to_string()),
            message: Some("Matching credential already imported.".to_string()),
        };
        assert!(import_result_requires_provider_restart(&already_imported));
    }

    #[test]
    fn import_result_restart_filter_ignores_non_mutating_statuses() {
        let unsupported = provider_auth_import::ProviderAuthImportResult {
            candidate_id: "cand-2".to_string(),
            provider_id: "cursor".to_string(),
            status: "unsupported".to_string(),
            profile_id: None,
            message: Some("Unsupported in this flow.".to_string()),
        };
        let error = provider_auth_import::ProviderAuthImportResult {
            candidate_id: "cand-3".to_string(),
            provider_id: "codex".to_string(),
            status: "error".to_string(),
            profile_id: None,
            message: Some("failed".to_string()),
        };
        assert!(!import_result_requires_provider_restart(&unsupported));
        assert!(!import_result_requires_provider_restart(&error));
    }

    #[test]
    fn should_skip_install_for_healthy_provider_without_updates() {
        let status = ctx_providers::adapters::ProviderStatus {
            provider_id: "codex".to_string(),
            installed: true,
            detected_path: None,
            version: Some("1.0.0".to_string()),
            capabilities: None,
            health: ctx_providers::adapters::ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
        };
        assert!(should_skip_install_for_healthy_provider(&status));
    }

    #[test]
    fn should_not_skip_install_for_healthy_provider_with_release_update() {
        let mut details = HashMap::new();
        details.insert("matrix_update_available".to_string(), "true".to_string());
        let status = ctx_providers::adapters::ProviderStatus {
            provider_id: "codex".to_string(),
            installed: true,
            detected_path: None,
            version: Some("1.0.0".to_string()),
            capabilities: None,
            health: ctx_providers::adapters::ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details,
        };
        assert!(!should_skip_install_for_healthy_provider(&status));
    }

    #[test]
    fn should_not_skip_install_for_healthy_provider_with_dependency_update() {
        let mut details = HashMap::new();
        details.insert(
            "managed_dependency_update_available".to_string(),
            "true".to_string(),
        );
        let status = ctx_providers::adapters::ProviderStatus {
            provider_id: "codex".to_string(),
            installed: true,
            detected_path: None,
            version: Some("1.0.0".to_string()),
            capabilities: None,
            health: ctx_providers::adapters::ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details,
        };
        assert!(!should_skip_install_for_healthy_provider(&status));
    }
}
