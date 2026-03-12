use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

mod handlers;

pub(in crate::api) use handlers::*;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::Json;
use chrono::Utc;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::errors::ApiErrorResp;
use super::provider_catalog::{
    provider_options_cache_entry_is_authoritative, runtime_probe_models_payload,
};
use super::provider_probe_auth::{provider_auth_mode, provider_has_active_auth_config};
use super::redact_json_value;
use crate::daemon::AppState;
use crate::harness_sources;
use crate::harness_sources::{
    HarnessApiShape, HarnessEndpointRecord, HarnessEndpointVerificationStatus, HarnessSourceKind,
};
use crate::installs::{InstallId, InstallInfo, InstallProgressEvent, InstallTarget};
use crate::logs;
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
