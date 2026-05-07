use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;

mod handlers;
mod provider_options_response;

pub(in crate::api) use handlers::*;
use provider_options_response::*;

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
    provider_options_cache_entry_is_authoritative, provider_supports_runtime_model_catalog,
    runtime_probe_models_payload,
};
use super::provider_probe_auth::provider_auth_mode;
use super::redact_json_value;
use crate::daemon::AppState;
use crate::logs;
use crate::provider_launch::install as provider_launch_install;
use crate::provider_launch::probe;
use crate::provider_launch::resolver::{
    is_acp_provider_id, runtime_probe_command_as_agent_command_for_target,
};
use crate::provider_launch::status::{install_target_for_workspace, provider_status_for_target};
use crate::provider_usability::{provider_status_is_usable, provider_status_unusable_reason};
use ctx_core::ids::WorkspaceId;
use ctx_harness_sources as harness_sources;
use ctx_harness_sources::{HarnessEndpointVerificationStatus, HarnessSourceKind};
use ctx_provider_install::install_state::{
    InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use ctx_provider_runtime::model_preferences::inject_preferred_model_id;
use ctx_provider_runtime::provider_auth::{
    selected_endpoint_from_harness_config, selected_endpoint_record_from_harness_config,
};
use ctx_provider_runtime::provider_launch::models::{
    endpoint_catalog_runtime_probe_failure, endpoint_catalog_verify_outcome,
    endpoint_models_payload, subscription_models_payload_from_status,
};
use ctx_provider_runtime::provider_launch::options::{
    endpoint_supports_model_catalog_verify, provider_options_probe_plan, ProviderOptionsProbePlan,
};
use ctx_provider_runtime::provider_launch::probe_error::classify_probe_error;
use ctx_providers::crp::{probe_crp_models, probe_crp_runtime_launch};

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

pub(in crate::api) async fn load_provider_source_config_with_error(
    data_root: &FsPath,
    provider_id: &str,
) -> (
    Option<harness_sources::HarnessProviderSourceConfig>,
    Option<String>,
) {
    if !harness_sources::supports_harness_endpoint(provider_id) {
        return (None, None);
    }

    match harness_sources::get_provider_source_config(data_root, provider_id).await {
        Ok(config) => (Some(config), None),
        Err(err) => (None, Some(logs::redact_sensitive(&err.to_string()))),
    }
}

pub(in crate::api) async fn load_managed_agent_server_config_with_error(
    data_root: &FsPath,
) -> (crate::installer::AgentServerConfigFile, Option<String>) {
    match crate::installer::load_agent_server_config(data_root).await {
        Ok(config) => (config, None),
        Err(err) => (
            crate::installer::AgentServerConfigFile::default(),
            Some(logs::redact_sensitive(&err.to_string())),
        ),
    }
}

struct PreparedProviderRuntimeProbe {
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    cwd: PathBuf,
    selected_endpoint_id: Option<String>,
}

fn workspace_execution_settings_error_json(
    error: &anyhow::Error,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": format!("failed to load workspace execution settings: {error:#}"),
        })),
    )
}

fn provider_install_error_response(
    error: provider_launch_install::StartProviderInstallError,
) -> (StatusCode, Json<serde_json::Value>) {
    let status = if error.code.as_deref() == Some("install_target_disabled") {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::BAD_REQUEST
    };
    (
        status,
        Json(serde_json::json!({
            "error": logs::redact_sensitive(&error.message),
            "code": error.code,
        })),
    )
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
    let (cfg, config_error) =
        load_managed_agent_server_config_with_error(&state.core.data_root).await;
    if let Some(config_error) = config_error {
        return Err(PreparedProviderRuntimeProbeError::Verify(config_error));
    }
    let runtime_command = runtime_probe_command_as_agent_command_for_target(
        &state.core.data_root,
        &cfg,
        provider_id,
        Some(install_target),
    )
    .map_err(|e| {
        PreparedProviderRuntimeProbeError::Verify(format!(
            "runtime_command_invalid: provider={provider_id} error={e}"
        ))
    })?
    .ok_or_else(|| {
        PreparedProviderRuntimeProbeError::Verify(format!(
            "runtime_command_missing: provider={provider_id} (configure an absolute runtime command)"
        ))
    })?;
    let command = runtime_command.command;
    let args = runtime_command.args;

    let probe_context =
        probe::provider_probe_context_for_workspace_runtime(state.as_ref(), workspace, provider_id)
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
    crate::installer::ensure_codex_cli_command_env_for_target(
        &mut env,
        &cfg,
        provider_id,
        Some(install_target),
    )
    .map_err(|e| {
        PreparedProviderRuntimeProbeError::Verify(format!(
            "codex_cli_command_invalid: provider={provider_id} error={e}"
        ))
    })?;

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

fn workspace_provider_cache_key(
    workspace_id: WorkspaceId,
    target: InstallTarget,
    provider_id: &str,
) -> String {
    format!("{}/{}/{}", workspace_id.0, target.as_str(), provider_id)
}

fn canonicalize_provider_id(provider_id: &str) -> String {
    provider_id.to_string()
}

fn project_provider_id_for_response(
    requested_provider_id: &str,
    canonical_provider_id_value: &str,
) -> String {
    let _ = requested_provider_id;
    canonical_provider_id_value.to_string()
}

fn project_provider_id_field(requested_provider_id: &str, value: &mut serde_json::Value) {
    let Some(provider_id) = value.get("provider_id").and_then(serde_json::Value::as_str) else {
        return;
    };
    let projected = project_provider_id_for_response(requested_provider_id, provider_id);
    if projected != provider_id {
        value["provider_id"] = serde_json::json!(projected);
    }
}

#[cfg(test)]
mod tests {
    use super::provider_install_error_response;
    use axum::http::StatusCode;

    #[test]
    fn provider_install_error_response_maps_disabled_install_targets_to_forbidden() {
        let (status, body) = provider_install_error_response(
            ctx_provider_runtime::provider_launch::install::StartProviderInstallError {
                message: "host provider installs are disabled by daemon policy".to_string(),
                code: Some("install_target_disabled".to_string()),
            },
        );

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body.0["code"], "install_target_disabled");
    }
}
