use std::collections::{HashMap, HashSet};
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::Json;
use chrono::Utc;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::errors::ApiErrorResp;
use super::redact_json_value;
use crate::daemon::AppState;
use crate::harness_sources;
use crate::harness_sources::{
    HarnessApiShape, HarnessEndpointRecord, HarnessEndpointVerificationStatus, HarnessSourceKind,
};
use crate::installs::{InstallId, InstallInfo, InstallProgressEvent, InstallTarget};
use crate::logs;
use crate::provider_accounts;
use crate::provider_launch::install as provider_launch_install;
use crate::provider_launch::probe;
use crate::provider_launch::resolver::{
    ensure_provider_adapter_for_target, is_acp_provider_id,
    runtime_probe_command_as_agent_command_for_target,
};
use crate::provider_launch::status::{
    install_target_for_workspace, provider_status_for_target,
    workspace_execution_settings_error_json,
};
use ctx_core::ids::WorkspaceId;
use ctx_providers::crp::{probe_crp_models, probe_crp_runtime_launch};

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

#[derive(Debug, Deserialize)]
pub(super) struct InstallTargetQuery {
    target: Option<String>,
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
    let auth_required = [
        "401",
        "403",
        "unauthorized",
        "forbidden",
        "authentication required",
        "auth required",
        "auth_required",
        "auth failed",
        "auth_failed",
        "auth error",
        "auth_error",
        "not authenticated",
        "not logged in",
        "login required",
        "sign in",
        "api key",
        "missing token",
        "invalid token",
        "expired token",
        "access token",
        "bearer token",
        "active account",
        "account env",
        "configure an active account",
    ];
    if auth_required.iter().any(|needle| lower.contains(needle)) {
        return (
            "auth_required",
            Some(true),
            HarnessEndpointVerificationStatus::Invalid,
        );
    }
    let protocol_error = [
        "invalid message",
        "invalid acp",
        "invalid crp",
        "models.list response",
        "models.list probe",
    ];
    if protocol_error.iter().any(|needle| lower.contains(needle)) {
        return (
            "error",
            Some(false),
            HarnessEndpointVerificationStatus::Error,
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

struct PreparedProviderRuntimeProbe {
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    cwd: PathBuf,
    selected_endpoint_id: Option<String>,
}

enum PreparedProviderRuntimeProbeError {
    Route((StatusCode, Json<serde_json::Value>)),
    Verify(String),
}

async fn prepare_provider_runtime_probe(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
    selected_endpoint_id: Option<String>,
) -> Result<PreparedProviderRuntimeProbe, PreparedProviderRuntimeProbeError> {
    let install_target = install_target_for_workspace(state, workspace.id)
        .await
        .map_err(|error| {
            PreparedProviderRuntimeProbeError::Route(workspace_execution_settings_error_json(
                &error,
            ))
        })?;
    let cfg = crate::installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    let runtime_command = runtime_probe_command_as_agent_command_for_target(
        &state.core.data_root,
        &cfg,
        provider_id,
        Some(install_target),
    )
    .map_err(|e| {
        PreparedProviderRuntimeProbeError::Route((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!(
                    "runtime_command_invalid: provider={provider_id} error={e}"
                ),
            })),
        ))
    })?
    .ok_or_else(|| {
        PreparedProviderRuntimeProbeError::Route((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!(
                    "runtime_command_missing: provider={provider_id} (configure an absolute runtime command)"
                ),
            })),
        ))
    })?;
    let command = runtime_command.command;
    let args = runtime_command.args;

    let probe_context =
        probe::provider_probe_context_for_workspace_runtime(state, workspace, provider_id)
            .await
            .map_err(PreparedProviderRuntimeProbeError::Verify)?;
    let source = probe_context.source;
    let mut env = probe_context.env;
    crate::installer::prepend_runtime_bin_dirs_to_provider_path_for_target(
        &mut env,
        &cfg,
        provider_id,
        &state.core.data_root,
        Some(install_target),
    );
    if is_acp_provider_id(provider_id) {
        crate::installer::prepend_runtime_bin_dirs_to_provider_path_for_target(
            &mut env,
            &cfg,
            "acp-crp-bridge",
            &state.core.data_root,
            Some(install_target),
        );
    }

    let selected_endpoint_id = if source.source_kind == HarnessSourceKind::Endpoint {
        source
            .endpoint
            .as_ref()
            .map(|endpoint| endpoint.id.clone())
            .or(selected_endpoint_id)
    } else {
        None
    };

    Ok(PreparedProviderRuntimeProbe {
        command,
        args,
        env,
        cwd: probe_context.cwd,
        selected_endpoint_id,
    })
}

pub(super) fn endpoint_catalog_runtime_probe_failure(
    message: String,
    endpoint_status: HarnessEndpointVerificationStatus,
) -> (
    String,
    Option<bool>,
    Option<String>,
    HarnessEndpointVerificationStatus,
) {
    let (status, auth_required, _) = classify_probe_error(&message);
    (
        status.to_string(),
        auth_required,
        Some(message),
        endpoint_status,
    )
}

fn workspace_provider_cache_key(
    workspace_id: WorkspaceId,
    target: InstallTarget,
    provider_id: &str,
) -> String {
    format!("{}/{}/{}", workspace_id.0, target.as_str(), provider_id)
}

pub(super) fn selected_endpoint_from_harness_config(
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

pub(crate) fn selected_endpoint_record_from_harness_config(
    config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> Option<HarnessEndpointRecord> {
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

pub(crate) fn subscription_models_payload_from_status(
    provider_status: &ctx_providers::adapters::ProviderStatus,
) -> Option<serde_json::Value> {
    crate::provider_accounts::pinned_subscription_models_value(
        &provider_status.provider_id,
        provider_status.version.as_deref(),
    )
}

fn endpoint_model_entries(endpoint: &HarnessEndpointRecord) -> Vec<serde_json::Value> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();

    for model in &endpoint.model_catalog_models {
        let id = model.id.trim();
        if id.is_empty() || !seen.insert(id.to_string()) {
            continue;
        }
        entries.push(serde_json::json!({
            "id": id,
            "name": model.name.clone(),
        }));
    }

    for model_id in &endpoint.manual_model_ids {
        let id = model_id.trim();
        if id.is_empty() || !seen.insert(id.to_string()) {
            continue;
        }
        entries.push(serde_json::json!({
            "id": id,
        }));
    }

    entries
}

fn endpoint_current_model_id(
    provider_id: &str,
    endpoint: &HarnessEndpointRecord,
) -> Option<String> {
    if provider_id == "droid" {
        return harness_sources::droid_cli_model_id_for_endpoint_model(
            endpoint.model_override.as_deref(),
            endpoint.base_url.as_deref(),
        );
    }
    endpoint
        .model_override
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn endpoint_models_payload(
    provider_id: &str,
    endpoint: &HarnessEndpointRecord,
    now: chrono::DateTime<chrono::Utc>,
) -> serde_json::Value {
    let stale = harness_sources::endpoint_model_catalog_is_stale(endpoint, now);
    serde_json::json!({
        "models": endpoint_model_entries(endpoint),
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

pub(super) fn endpoint_supports_model_catalog_verify(endpoint: &HarnessEndpointRecord) -> bool {
    endpoint.api_shape == HarnessApiShape::OpenaiResponses
        && endpoint
            .base_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty())
}

pub(super) fn endpoint_catalog_verify_outcome(
    endpoint: &HarnessEndpointRecord,
) -> (
    String,
    Option<bool>,
    Option<String>,
    HarnessEndpointVerificationStatus,
) {
    match endpoint.model_catalog_status {
        harness_sources::EndpointModelCatalogStatus::Ready
        | harness_sources::EndpointModelCatalogStatus::ManualOnly => (
            "ok".to_string(),
            Some(false),
            None,
            HarnessEndpointVerificationStatus::Valid,
        ),
        harness_sources::EndpointModelCatalogStatus::Unknown
        | harness_sources::EndpointModelCatalogStatus::Error => {
            let detail = endpoint
                .model_catalog_error
                .as_ref()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| {
                    "endpoint model catalog is unavailable; refresh endpoint models in Settings"
                        .to_string()
                });
            let redacted = logs::redact_sensitive(&detail);
            let (status, auth_required, endpoint_status) = classify_probe_error(&redacted);
            (
                status.to_string(),
                auth_required,
                Some(redacted),
                endpoint_status,
            )
        }
    }
}

async fn provider_has_active_auth_config(
    data_root: &StdPath,
    provider_id: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> bool {
    if let Some(config) = source_config {
        if endpoint_selection_is_active(config) {
            return true;
        }
        if provider_id == "codex" && config.selected_source_kind == HarnessSourceKind::Subscription
        {
            return true;
        }
    }
    match provider_accounts::subscription_env_for_active_account(data_root, provider_id).await {
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
    const CACHE_TTL: Duration = Duration::from_secs(30);
    const VERIFY_TTL: Duration = Duration::from_secs(30 * 60);

    if provider_id == "codex-crp" {
        return Err(invalid_provider_id_error("codex-crp", "codex"));
    }

    let ws_id = parse_workspace_id(&ws_id)?;
    let install_target = install_target_for_workspace(&state, ws_id)
        .await
        .map_err(|error| workspace_execution_settings_error_json(&error))?;
    let cache_key = workspace_provider_cache_key(ws_id, install_target, &provider_id);
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

    let managed = crate::installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    let known = {
        let map = state.providers.statuses.lock().await;
        map.contains_key(&provider_id)
            || crate::provider_matrix::get_entry(&matrix, &provider_id).is_some()
    };
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

    if !known {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unsupported provider id: {provider_id}"),
            })),
        ));
    }

    let provider_status =
        provider_status_for_target(&state, &managed, &matrix, &provider_id, install_target).await;

    if !provider_status.installed
        || !matches!(
            provider_status.health,
            ctx_providers::adapters::ProviderHealth::Ok
        )
    {
        let mut raw_base_resp = serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": ws_id.0,
            "installed": provider_status.installed,
            "health": provider_status.health,
            "diagnostics": provider_status.diagnostics,
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

    let use_crp_probe = provider_id == "codex" || provider_id == "claude-crp";
    if !use_crp_probe {
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
        let (probe_ok, auth_required, probe_error) =
            match probe::provider_probe_env_for_workspace_runtime(&state, &ws, &provider_id).await {
                Ok(_) => (true, false, None),
                Err(err) => {
                    let probe_error = logs::redact_sensitive(&err);
                    let (_, auth_required, _) = classify_probe_error(&probe_error);
                    (false, auth_required.unwrap_or(false), Some(probe_error))
                }
            };
        let now = chrono::Utc::now();
        let mut raw_resp = serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": ws_id.0,
            "installed": provider_status.installed,
            "probe_ok": probe_ok,
            "supports_load": false,
            "auth_required": auth_required,
            "has_active_auth": has_active_auth,
            "auth_mode": auth_mode,
            "probed_at": now.to_rfc3339(),
        });
        if let Some(probe_error) = probe_error {
            raw_resp["probe_error"] = serde_json::json!(probe_error);
        }
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
        } else if let Some(models) = subscription_models_payload_from_status(&provider_status) {
            raw_resp["models"] = models;
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
            "installed": provider_status.installed,
            "probe_ok": true,
            "supports_load": false,
            "auth_required": false,
            "has_active_auth": has_active_auth,
            "auth_mode": auth_mode,
            "models": endpoint_models_payload(&provider_id, endpoint, now),
            "probed_at": now.to_rfc3339(),
        });
        if let Some(source) = source_config.as_ref() {
            raw_resp["source"] = serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
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

    let probe = match prepare_provider_runtime_probe(&state, &ws, &provider_id, None).await {
        Ok(prepared) => {
            probe_crp_models(
                &provider_id,
                prepared.command,
                prepared.args,
                prepared.cwd,
                prepared.env,
            )
            .await
        }
        Err(PreparedProviderRuntimeProbeError::Route(err)) => return Err(err),
        Err(PreparedProviderRuntimeProbeError::Verify(err)) => Err(anyhow::anyhow!(err)),
    };

    let mut raw_resp = match probe {
        Ok(probe) => serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": ws_id.0,
            "installed": provider_status.installed,
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
        Err(e) => {
            let probe_error = logs::redact_sensitive(&e.to_string());
            let (_, auth_required, _) = classify_probe_error(&probe_error);
            serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": ws_id.0,
                "installed": provider_status.installed,
                "probe_ok": false,
                "probe_error": probe_error,
                "auth_required": auth_required.unwrap_or(false),
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "probed_at": chrono::Utc::now().to_rfc3339(),
            })
        }
    };
    if let Some(source) = source_config.as_ref() {
        raw_resp["source"] = serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
    }
    if raw_resp.get("models").is_none() || raw_resp.get("models").is_some_and(|v| v.is_null()) {
        if let Some(models) = subscription_models_payload_from_status(&provider_status) {
            raw_resp["models"] = models;
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
    Ok(Json(out))
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

    let install_target = install_target_for_workspace(&state, workspace.id)
        .await
        .map_err(|error| workspace_execution_settings_error_json(&error))?;
    let managed = crate::installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    let known = {
        let map = state.providers.statuses.lock().await;
        map.contains_key(&provider_id)
            || crate::provider_matrix::get_entry(&matrix, &provider_id).is_some()
    };
    if !known {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unsupported provider id: {provider_id}"),
            })),
        ));
    }
    let provider_status =
        provider_status_for_target(&state, &managed, &matrix, &provider_id, install_target).await;

    let checked_at = Utc::now().to_rfc3339();
    let mut status = "ok".to_string();
    let mut auth_required = Some(false);
    let mut message: Option<String> = None;
    let mut endpoint_status = HarnessEndpointVerificationStatus::Valid;
    let source_config =
        harness_sources::get_provider_source_config(&state.core.data_root, &provider_id)
            .await
            .ok();
    let selected_endpoint = selected_endpoint_record_from_harness_config(source_config.as_ref());
    let mut selected_endpoint_id: Option<String> =
        selected_endpoint_from_harness_config(source_config);

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
    } else if let Some(endpoint) = selected_endpoint
        .as_ref()
        .filter(|endpoint| endpoint_supports_model_catalog_verify(endpoint))
    {
        match harness_sources::refresh_provider_endpoint_model_catalog(
            &state.core.data_root,
            &provider_id,
            &endpoint.id,
        )
        .await
        {
            Ok(refreshed_endpoint) => {
                selected_endpoint_id = Some(refreshed_endpoint.id.clone());
                let (next_status, next_auth, next_message, next_endpoint_status) =
                    endpoint_catalog_verify_outcome(&refreshed_endpoint);
                status = next_status;
                auth_required = next_auth;
                message = next_message;
                endpoint_status = next_endpoint_status;
            }
            Err(err) => {
                let msg = logs::redact_sensitive(&err.to_string());
                let (classified, auth, endpoint_verify) = classify_probe_error(&msg);
                status = classified.to_string();
                auth_required = auth;
                message = Some(msg);
                endpoint_status = endpoint_verify;
            }
        }

        if status == "ok" {
            match prepare_provider_runtime_probe(
                &state,
                &workspace,
                &provider_id,
                selected_endpoint_id.clone(),
            )
            .await
            {
                Ok(prepared) => {
                    selected_endpoint_id = prepared.selected_endpoint_id;
                    if let Err(err) = probe_crp_runtime_launch(
                        &provider_id,
                        prepared.command,
                        prepared.args,
                        prepared.cwd,
                        prepared.env,
                    )
                    .await
                    {
                        let msg = logs::redact_sensitive(&err.to_string());
                        let (next_status, next_auth, next_message, next_endpoint_status) =
                            endpoint_catalog_runtime_probe_failure(msg, endpoint_status);
                        status = next_status;
                        auth_required = next_auth;
                        message = next_message;
                        endpoint_status = next_endpoint_status;
                    }
                }
                Err(PreparedProviderRuntimeProbeError::Route(err)) => return Err(err),
                Err(PreparedProviderRuntimeProbeError::Verify(err)) => {
                    let msg = logs::redact_sensitive(&err);
                    let (next_status, next_auth, next_message, next_endpoint_status) =
                        endpoint_catalog_runtime_probe_failure(msg, endpoint_status);
                    status = next_status;
                    auth_required = next_auth;
                    message = next_message;
                    endpoint_status = next_endpoint_status;
                }
            }
        }
    } else {
        match prepare_provider_runtime_probe(
            &state,
            &workspace,
            &provider_id,
            selected_endpoint_id.clone(),
        )
        .await
        {
            Ok(prepared) => {
                selected_endpoint_id = prepared.selected_endpoint_id;
                let probe = probe_crp_models(
                    &provider_id,
                    prepared.command,
                    prepared.args,
                    prepared.cwd,
                    prepared.env,
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
            Err(PreparedProviderRuntimeProbeError::Route(err)) => return Err(err),
            Err(PreparedProviderRuntimeProbeError::Verify(err)) => {
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
    let cache_key = workspace_provider_cache_key(ws_id, install_target, &provider_id);
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

    let probe_context =
        probe::provider_auth_context_for_workspace_runtime(&state, &workspace, &provider_id)
            .await
            .map_err(|err| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": err,
                    })),
                )
            })?;
    if probe_context.source.source_kind == HarnessSourceKind::Endpoint {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "selected source is endpoint; update endpoint key/config directly instead of interactive authenticate",
            })),
        ));
    }

    let install_target = install_target_for_workspace(&state, workspace.id)
        .await
        .map_err(|error| workspace_execution_settings_error_json(&error))?;
    let adapter =
        ensure_provider_adapter_for_target(state.as_ref(), &provider_id, install_target).await;

    let (event_tx, mut event_rx) = mpsc::channel(32);
    tokio::spawn(async move { while event_rx.recv().await.is_some() {} });

    let checked_at = Utc::now().to_rfc3339();
    let result = adapter
        .authenticate_session(
            format!("auth-{}", uuid::Uuid::new_v4()),
            probe_context.cwd,
            probe_context.env,
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
    let cache_key = workspace_provider_cache_key(ws_id, install_target, &provider_id);
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
    let target = crate::installer::parse_install_target(query.target.as_deref()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": e.to_string()
            })),
        )
    })?;

    let (install_id, _) =
        provider_launch_install::start_provider_install(&state, &id, target).await?;

    Ok(Json(InstallStartResponse {
        provider_id: id,
        install_id,
        target,
    }))
}

pub(super) async fn install_all_providers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<Vec<InstallStartResponse>>, StatusCode> {
    let target = crate::installer::parse_install_target(query.target.as_deref())
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let installs = provider_launch_install::start_all_provider_installs(&state, target).await;
    Ok(Json(
        installs
            .into_iter()
            .map(|(provider_id, install_id)| InstallStartResponse {
                provider_id,
                install_id,
                target,
            })
            .collect(),
    ))
}

pub(super) async fn get_install(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Json<InstallInfo>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .get_install_polling_info(install_id)
        .await
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Debug, Deserialize)]
pub(super) struct GetInstallStatusesReq {
    pub(super) install_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct InstallStatusBatchItem {
    pub(super) install_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) info: Option<InstallInfo>,
}

#[derive(Debug, Serialize)]
pub(super) struct GetInstallStatusesResp {
    pub(super) installs: Vec<InstallStatusBatchItem>,
}

pub(super) async fn get_install_statuses(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GetInstallStatusesReq>,
) -> Result<Json<GetInstallStatusesResp>, (StatusCode, Json<ApiErrorResp>)> {
    let install_ids = req
        .install_ids
        .into_iter()
        .map(|raw| {
            let parsed = uuid::Uuid::parse_str(raw.trim()).map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("invalid install id: {raw}"),
                    }),
                )
            })?;
            Ok(InstallId::from(parsed))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut installs = Vec::with_capacity(install_ids.len());
    for install_id in install_ids {
        let info = state.get_install_polling_info(install_id).await;
        installs.push(InstallStatusBatchItem {
            install_id: install_id.to_string(),
            info,
        });
    }

    Ok(Json(GetInstallStatusesResp { installs }))
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

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
