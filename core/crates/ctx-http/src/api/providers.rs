use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::Json;
use chrono::{DateTime, Utc};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use url::Url;

use super::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::harness_sources;
use crate::harness_sources::{
    HarnessApiShape, HarnessEndpointUpsert, HarnessEndpointVerificationStatus, HarnessSourceKind,
};
use crate::installer;
use crate::installs::{InstallId, InstallInfo, InstallProgressEvent};
use crate::logs;
use crate::provider_accounts;
use crate::provider_auth_import;
use crate::provider_usage;
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

pub(super) async fn list_providers(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ProviderStatus>>, StatusCode> {
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
            if installer::is_supported_managed_provider(&matrix, &status.provider_id) {
                "true".into()
            } else {
                "false".into()
            },
        );
        if let Some(install_id) = state.find_running_install(&status.provider_id).await {
            status
                .details
                .insert("install_running".into(), "true".into());
            status
                .details
                .insert("install_id".into(), install_id.to_string());
        }
    }
    Ok(Json(out))
}

pub(super) async fn get_provider(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
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
        if installer::is_supported_managed_provider(&matrix, &status.provider_id) {
            "true".into()
        } else {
            "false".into()
        },
    );
    if let Some(install_id) = state.find_running_install(&id).await {
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
pub(super) struct KimiAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::KimiAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct CopilotAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CopilotAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct KiroAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::KiroAccountEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct CursorAccountsResponse {
    active_account_id: Option<String>,
    accounts: Vec<provider_accounts::CursorAccountEntry>,
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
    auth_token: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClaudeActiveAccountReq {
    account_id: Option<String>,
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
pub(super) struct GeminiActiveAccountReq {
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
pub(super) struct KiroAccountUpsertReq {
    label: Option<String>,
    auth_token_json: String,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KiroActiveAccountReq {
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

const CODEX_LOGIN_RPC_TIMEOUT: Duration = Duration::from_secs(30);

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

async fn kimi_accounts_response(state: &Arc<AppState>) -> KimiAccountsResponse {
    let registry = provider_accounts::load_kimi_registry(&state.core.data_root).await;
    KimiAccountsResponse {
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

async fn kiro_accounts_response(state: &Arc<AppState>) -> KiroAccountsResponse {
    let registry = provider_accounts::load_kiro_registry(&state.core.data_root).await;
    KiroAccountsResponse {
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

async fn restart_kimi_providers_for_auth_change(state: &Arc<AppState>, reason: &str) {
    restart_provider_for_auth_change(state, "kimi", reason).await;
}

async fn restart_copilot_providers_for_auth_change(state: &Arc<AppState>, reason: &str) {
    restart_provider_for_auth_change(state, "copilot", reason).await;
}

async fn restart_kiro_providers_for_auth_change(state: &Arc<AppState>, reason: &str) {
    restart_provider_for_auth_change(state, "kiro", reason).await;
}

async fn restart_cursor_providers_for_auth_change(state: &Arc<AppState>, reason: &str) {
    restart_provider_for_auth_change(state, "cursor", reason).await;
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

pub(super) async fn upsert_claude_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClaudeAccountUpsertReq>,
) -> Result<Json<ClaudeAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_claude_account(&state.core.data_root, req.label, req.auth_token)
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

pub(super) async fn list_kiro_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<KiroAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    Ok(Json(kiro_accounts_response(&state).await))
}

pub(super) async fn upsert_kiro_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<KiroAccountUpsertReq>,
) -> Result<Json<KiroAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::add_kiro_account(
        &state.core.data_root,
        req.label,
        req.auth_token_json,
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
    restart_kiro_providers_for_auth_change(&state, "kiro auth updated").await;
    Ok(Json(kiro_accounts_response(&state).await))
}

pub(super) async fn set_kiro_active_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<KiroActiveAccountReq>,
) -> Result<Json<KiroAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(ref account_id) = req.account_id {
        let registry = provider_accounts::load_kiro_registry(&state.core.data_root).await;
        if !registry.accounts.iter().any(|a| a.id == *account_id) {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "unknown account".to_string(),
                }),
            ));
        }
    }
    provider_accounts::set_active_kiro_account(&state.core.data_root, req.account_id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_kiro_providers_for_auth_change(&state, "kiro auth updated").await;
    Ok(Json(kiro_accounts_response(&state).await))
}

pub(super) async fn delete_kiro_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<KiroAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    provider_accounts::remove_kiro_account(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    restart_kiro_providers_for_auth_change(&state, "kiro auth updated").await;
    Ok(Json(kiro_accounts_response(&state).await))
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
    let codex_mutated = results.iter().any(|result| {
        result.provider_id == "codex" && matches!(result.status.as_str(), "imported" | "updated")
    });
    if codex_mutated {
        restart_codex_providers_for_auth_change(&state, "codex auth updated").await;
    }
    Ok(Json(ProviderAuthImportResponse { results }))
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
    model_override: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
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

async fn provider_probe_env(
    state: &Arc<AppState>,
    provider_id: &str,
) -> Result<
    (
        harness_sources::ResolvedHarnessSource,
        HashMap<String, String>,
    ),
    String,
> {
    let source =
        harness_sources::resolve_provider_source_for_probe(&state.core.data_root, provider_id)
            .await
            .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    let mut env = HashMap::new();
    env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    if let Some(token) = state.core.auth_token.as_ref() {
        env.insert("CTX_AUTH_TOKEN".to_string(), token.clone());
    }
    if source.source_kind == HarnessSourceKind::Subscription {
        let extra = crate::provider_accounts::subscription_env_for_active_account(
            &state.core.data_root,
            provider_id,
        )
        .await;
        if let Ok(extra) = extra {
            for (key, value) in extra {
                env.insert(key, value);
            }
        }
    }
    for (key, value) in source.env.iter() {
        env.insert(key.clone(), value.clone());
    }
    Ok((source, env))
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
        let mut raw_resp = serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": ws_id.0,
            "installed": provider_status.as_ref().map(|s| s.installed).unwrap_or(true),
            "probe_ok": true,
            "supports_load": false,
            "auth_required": false,
            "probed_at": chrono::Utc::now().to_rfc3339(),
        });
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
        let command = runtime_command.command_abs_path;
        let args = runtime_command.args;

        let probe = match provider_probe_env(&state, &provider_id).await {
            Ok((_source, env)) => {
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
    harness_sources::upsert_provider_endpoint(
        &state.core.data_root,
        &id,
        HarnessEndpointUpsert {
            endpoint_id: req.endpoint_id,
            name: req.name,
            base_url: req.base_url,
            api_shape: req.api_shape,
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
        let command = runtime_command.command_abs_path;
        let args = runtime_command.args;

        match provider_probe_env(&state, &provider_id).await {
            Ok((source, env)) => {
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

    let (source, provider_env) = provider_probe_env(&state, &provider_id)
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
) -> Result<Json<InstallStartResponse>, (StatusCode, Json<serde_json::Value>)> {
    if id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    if !installer::is_supported_managed_provider(&matrix, &id) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unsupported provider for managed install: {id}")
            })),
        ));
    }

    let (install_id, started_new) = state.start_install(id.clone()).await;
    if started_new {
        let state2 = state.clone();
        let provider_id = id.clone();
        tokio::spawn(async move {
            if let Err(e) = installer::install_provider_with_progress(
                state2.clone(),
                install_id,
                provider_id.clone(),
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
    let (install_id, started_new) = state.start_install(install_key).await;
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
) -> Result<Json<Vec<InstallStartResponse>>, StatusCode> {
    let mut out = Vec::new();
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    for entry in matrix.providers.iter() {
        if !installer::is_supported_managed_provider(&matrix, &entry.id) {
            continue;
        }
        let id = entry.id.as_str();
        if let Some(install_id) = state.find_running_install(id).await {
            out.push(InstallStartResponse {
                provider_id: id.to_string(),
                install_id,
            });
            continue;
        }

        let status = state.providers.statuses.lock().await.get(id).cloned();
        if let Some(st) = status {
            if st.installed && matches!(st.health, ctx_providers::adapters::ProviderHealth::Ok) {
                continue;
            }
        }

        let (install_id, started_new) = state.start_install(id.to_string()).await;
        if started_new {
            let state2 = state.clone();
            let provider_id = id.to_string();
            tokio::spawn(async move {
                if let Err(e) = installer::install_provider_with_progress(
                    state2.clone(),
                    install_id,
                    provider_id.clone(),
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
        });
    }
    Ok(Json(out))
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
}
