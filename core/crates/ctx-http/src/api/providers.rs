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

use super::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::installer;
use crate::installs::{InstallId, InstallInfo, InstallProgressEvent};
use crate::logs;
use crate::provider_accounts;
use crate::provider_usage;
use ctx_core::ids::WorkspaceId;
use ctx_providers::adapters::{ProviderRestartMode, ProviderStatus};
use ctx_providers::crp::probe_crp_models;

use super::redact_json_value;
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
) -> Result<Json<ProviderStatus>, StatusCode> {
    let map = state.providers.statuses.lock().await;
    let mut status = map.get(&id).cloned().ok_or(StatusCode::NOT_FOUND)?;
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
) -> Result<Json<provider_usage::ProviderUsageSnapshot>, (StatusCode, Json<ApiErrorResp>)> {
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
                            Json(ApiErrorResp {
                                error: e.to_string(),
                            }),
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
                        Json(ApiErrorResp {
                            error: e.to_string(),
                        }),
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
}

#[derive(Debug, Deserialize)]
pub(super) struct CodexActiveAccountReq {
    account_id: Option<String>,
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

pub(super) async fn list_codex_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CodexAccountsResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let registry = provider_accounts::load_codex_registry(&state.core.data_root).await;
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
    let status = provider_accounts::CodexLoginStatus {
        account_id: account_id.clone(),
        auth_url: login.auth_url.clone(),
        status: "pending".to_string(),
        error: None,
    };
    {
        let mut map = state.providers.codex_login_sessions.lock().await;
        map.insert(account_id.clone(), status);
    }
    let state_clone = Arc::clone(&state);
    let account_id_for_task = account_id.clone();
    let auth_url = login.auth_url.clone();
    tokio::spawn(async move {
        monitor_codex_login(state_clone, account_id_for_task, label, login).await;
    });

    Ok(Json(CodexLoginStartResp {
        account_id,
        auth_url,
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
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: e.to_string(),
                    }),
                )
            })?;
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
            email,
            plan_type,
            created_at: Utc::now(),
            last_used_at: Some(Utc::now()),
        };
        if provider_accounts::upsert_codex_account(&state.core.data_root, entry)
            .await
            .is_ok()
        {
            let _ = provider_accounts::set_active_codex_account(
                &state.core.data_root,
                Some(account_id.clone()),
            )
            .await;
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

pub(super) fn default_agent_server_command(
    matrix: &crate::provider_matrix::ProviderMatrix,
    data_root: &std::path::Path,
    provider_id: &str,
) -> Option<(String, Vec<String>)> {
    let entry = crate::provider_matrix::get_entry(matrix, provider_id)?;
    let mut cmd = entry.command.clone()?;
    if provider_id == "cagent" {
        cmd.args.push(
            crate::installer::cagent_config_path(data_root)
                .to_string_lossy()
                .to_string(),
        );
    }
    Some((cmd.command, cmd.args))
}

fn is_crp_provider(provider_id: &str) -> bool {
    provider_id.ends_with("-crp") || matches!(provider_id, "codex" | "claude")
}

fn is_codex_provider(provider_id: &str) -> bool {
    matches!(provider_id, "codex" | "codex-crp")
}

pub(super) async fn get_provider_options(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiErrorResp>)> {
    const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);
    const VERIFY_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);

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

    if let Some(st) = provider_status.as_ref() {
        if !st.installed || !matches!(st.health, ctx_providers::adapters::ProviderHealth::Ok) {
            let base_resp = redact_json_value(serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id.0,
                "installed": st.installed,
                "health": st.health,
                "diagnostics": st.diagnostics,
                "probe_ok": false,
                "probe_error": "provider not installed or unhealthy",
                "probed_at": chrono::Utc::now().to_rfc3339(),
            }));
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

    let use_crp_probe = is_crp_provider(&provider_id);
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
                    Json(ApiErrorResp {
                        error: "failed to load workspace".to_string(),
                    }),
                )
            })?
            .ok_or((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "workspace not found".to_string(),
                }),
            ))?;

        let cfg = installer::load_agent_server_config(&state.core.data_root)
            .await
            .unwrap_or_default();
        let matrix = crate::provider_matrix::load_matrix_cached(
            &state.core.data_root,
            &state.providers.matrix_cache,
        )
        .await;
        let (command, args) = cfg
            .providers
            .get(&provider_id)
            .map(|c| (c.command.clone(), c.args.clone()))
            .or_else(|| default_agent_server_command(&matrix, &state.core.data_root, &provider_id))
            .ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "unknown provider id".to_string(),
                }),
            ))?;

        let mut env = std::collections::HashMap::new();
        env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
        if let Some(token) = state.core.auth_token.as_ref() {
            env.insert("CTX_AUTH_TOKEN".to_string(), token.clone());
        }
        if is_codex_provider(&provider_id) {
            // codex-crp relies on Codex auth material (via CODEX_HOME). Without it, probing can
            // return empty models even when Codex is otherwise configured.
            if let Ok(extra) =
                crate::provider_accounts::codex_env_for_active_account(&state.core.data_root).await
            {
                for (key, value) in extra {
                    env.insert(key, value);
                }
            }
        }

        let probe = probe_crp_models(
            &provider_id,
            command,
            args,
            PathBuf::from(&ws.root_path),
            env,
        )
        .await;

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
    unreachable!("provider options probe fell through");
}

#[derive(Debug, Deserialize)]
pub(super) struct AuthenticateProviderReq {
    #[serde(default)]
    method_id: Option<String>,
}

pub(super) async fn authenticate_provider_for_workspace(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
    Json(_req): Json<AuthenticateProviderReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);

    let _ws = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load workspace".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;
    if !is_crp_provider(&provider_id) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "provider does not support authentication".to_string(),
            }),
        ));
    }

    let resp = redact_json_value(serde_json::json!({
        "provider_id": provider_id,
        "workspace_id": ws_id.0,
        "status": "ok",
        "auth_required": false,
        "auth_methods": null,
        "acp_error": null,
        "checked_at": chrono::Utc::now().to_rfc3339(),
    }));
    Ok(Json(resp))
}

pub(super) async fn verify_provider_for_workspace(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);

    let _ws = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load workspace".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;
    if !is_crp_provider(&provider_id) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "provider does not support verification".to_string(),
            }),
        ));
    }

    let resp = redact_json_value(serde_json::json!({
        "provider_id": provider_id.clone(),
        "workspace_id": ws_id.0,
        "status": "ok",
        "auth_required": false,
        "auth_methods": null,
        "acp_error": null,
        "checked_at": chrono::Utc::now().to_rfc3339(),
    }));

    let cache_key = format!("{}/{}", ws_id.0, provider_id);
    state.providers.verify_cache.lock().await.insert(
        cache_key,
        crate::daemon::CachedProviderVerify {
            cached_at: std::time::Instant::now(),
            value: resp.clone(),
        },
    );

    Ok(Json(resp))
}

pub(super) async fn install_provider(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<InstallStartResponse>, StatusCode> {
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    if !installer::is_supported_managed_provider(&matrix, &id) {
        return Err(StatusCode::BAD_REQUEST);
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
