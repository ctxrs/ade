use std::sync::Arc;

use axum::http::StatusCode;
use axum::Json;
use ctx_provider_runtime::provider_launch::runtime_probe::{
    prepare_provider_runtime_probe_launch, PreparedProviderRuntimeProbe,
};

use crate::api::provider_launch::{
    load_managed_agent_server_config_with_error, workspace_execution_settings_error_json,
};
use crate::daemon::providers::install_target_for_workspace;
use crate::daemon::AppState;
use ctx_provider_runtime::provider_launch::probe;

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
    let probe_context =
        probe::provider_probe_context_for_workspace_runtime(state.as_ref(), workspace, provider_id)
            .await
            .map_err(PreparedProviderRuntimeProbeError::Verify)?;
    prepare_provider_runtime_probe_launch(
        &state.core.data_root,
        &cfg,
        provider_id,
        install_target,
        probe_context,
        selected_endpoint_id,
    )
    .map_err(|error| PreparedProviderRuntimeProbeError::Verify(error.into_message()))
}
