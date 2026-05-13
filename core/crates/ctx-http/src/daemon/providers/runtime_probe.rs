use std::sync::Arc;

use ctx_provider_runtime::provider_launch::probe;
use ctx_provider_runtime::provider_launch::runtime_probe::{
    prepare_provider_runtime_probe_launch, PreparedProviderRuntimeProbe,
};

use crate::daemon::providers::install_target_for_workspace;
use crate::daemon::AppState;

pub(crate) enum PreparedProviderRuntimeProbeError {
    ExecutionSettings(anyhow::Error),
    Verify(String),
}

pub(crate) async fn prepare_provider_runtime_probe(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
    selected_endpoint_id: Option<String>,
) -> Result<PreparedProviderRuntimeProbe, PreparedProviderRuntimeProbeError> {
    let install_target = install_target_for_workspace(state, workspace.id)
        .await
        .map_err(PreparedProviderRuntimeProbeError::ExecutionSettings)?;
    let (cfg, config_error) =
        ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
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
