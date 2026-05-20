use std::sync::Arc;

use ctx_provider_runtime::provider_runtime_probe_service as runtime_probe_service;
pub use ctx_provider_runtime::provider_runtime_probe_service::ProviderAuthVerificationRuntimeProbe;

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
