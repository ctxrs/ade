use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::Json;
use ctx_harness_sources::HarnessSourceKind;
use ctx_provider_runtime::provider_launch::resolver::{
    is_acp_provider_id, runtime_probe_command_as_agent_command_for_target,
};

use crate::api::provider_launch::{
    load_managed_agent_server_config_with_error, workspace_execution_settings_error_json,
};
use crate::daemon::provider_launch::probe;
use crate::daemon::provider_launch::status::install_target_for_workspace;
use crate::daemon::AppState;

pub(in crate::api::provider_launch) struct PreparedProviderRuntimeProbe {
    pub(in crate::api::provider_launch) command: String,
    pub(in crate::api::provider_launch) args: Vec<String>,
    pub(in crate::api::provider_launch) env: HashMap<String, String>,
    pub(in crate::api::provider_launch) cwd: PathBuf,
    pub(in crate::api::provider_launch) selected_endpoint_id: Option<String>,
}

pub(in crate::api::provider_launch) enum PreparedProviderRuntimeProbeError {
    Route((StatusCode, Json<serde_json::Value>)),
    Verify(String),
}

pub(in crate::api::provider_launch) async fn prepare_provider_runtime_probe(
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
    crate::daemon::installer::prepend_runtime_bin_dirs_to_provider_path_for_target(
        &mut env,
        &cfg,
        provider_id,
        &state.core.data_root,
        Some(install_target),
    );
    if is_acp_provider_id(provider_id) {
        crate::daemon::installer::prepend_runtime_bin_dirs_to_provider_path_for_target(
            &mut env,
            &cfg,
            "acp-crp-bridge",
            &state.core.data_root,
            Some(install_target),
        );
    }
    crate::daemon::installer::ensure_codex_cli_command_env_for_target(
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
