use std::sync::Arc;

use ctx_provider_runtime::provider_runtime_probe_service as runtime_probe_service;
pub use ctx_provider_runtime::provider_runtime_probe_service::{
    ProviderAuthVerificationRuntimeProbe, ProviderRuntimeProbeStatus,
};
use ctx_providers::crp::CrpModelsProbe;

use crate::daemon::providers::install_target_for_workspace;
use crate::daemon::DaemonState;

pub async fn probe_provider_auth_verification_runtime(
    state: &Arc<DaemonState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
    selected_endpoint_id: Option<String>,
) -> Result<ProviderAuthVerificationRuntimeProbe, anyhow::Error> {
    let install_target = install_target_for_workspace(state, workspace.id).await?;
    runtime_probe_service::probe_provider_auth_verification_runtime(
        state.as_ref(),
        workspace,
        provider_id,
        install_target,
        selected_endpoint_id,
    )
    .await
    .map_err(|error| match error {
        runtime_probe_service::PreparedProviderRuntimeProbeError::Verify(error) => {
            anyhow::anyhow!(error)
        }
    })
}

pub async fn provider_has_active_auth_for_workspace_runtime(
    state: &Arc<DaemonState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
    source_config: Option<&ctx_harness_sources::HarnessProviderSourceConfig>,
) -> Result<bool, String> {
    runtime_probe_service::provider_has_active_auth_for_workspace_runtime(
        state.as_ref(),
        workspace,
        provider_id,
        source_config,
    )
    .await
}

pub async fn probe_provider_options_env(
    state: &Arc<DaemonState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
) -> ProviderRuntimeProbeStatus {
    runtime_probe_service::probe_provider_options_env(state.as_ref(), workspace, provider_id).await
}

pub async fn probe_selected_endpoint_runtime_launch(
    state: &Arc<DaemonState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
    endpoint_id: String,
) -> Result<ProviderRuntimeProbeStatus, anyhow::Error> {
    let install_target = install_target_for_workspace(state, workspace.id).await?;
    runtime_probe_service::probe_selected_endpoint_runtime_launch(
        state.as_ref(),
        workspace,
        provider_id,
        install_target,
        endpoint_id,
    )
    .await
    .map_err(|error| match error {
        runtime_probe_service::PreparedProviderRuntimeProbeError::Verify(error) => {
            anyhow::anyhow!(error)
        }
    })
}

pub async fn probe_runtime_models_for_provider_options(
    state: &Arc<DaemonState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
) -> Result<anyhow::Result<CrpModelsProbe>, anyhow::Error> {
    let install_target = install_target_for_workspace(state, workspace.id).await?;
    Ok(
        runtime_probe_service::probe_runtime_models_for_provider_options(
            state.as_ref(),
            workspace,
            provider_id,
            install_target,
        )
        .await,
    )
}
