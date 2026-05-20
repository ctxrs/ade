use std::sync::Arc;

use ctx_core::models::Workspace;
use ctx_provider_runtime::provider_options::service::effective_preferred_model_id_for_workspace_runtime;

use crate::daemon::providers;
use crate::daemon::DaemonState;

#[derive(Debug)]
pub enum EffectivePreferredModelError {
    ExecutionSettings(anyhow::Error),
}

pub async fn effective_preferred_model_id_for_workspace(
    state: &Arc<DaemonState>,
    workspace: &Workspace,
    provider_id: &str,
    preferred_model_id: Option<String>,
) -> Result<Option<String>, EffectivePreferredModelError> {
    let install_target = providers::install_target_for_workspace(state, workspace.id)
        .await
        .map_err(EffectivePreferredModelError::ExecutionSettings)?;
    Ok(effective_preferred_model_id_for_workspace_runtime(
        state.as_ref(),
        workspace,
        provider_id,
        install_target,
        preferred_model_id,
    )
    .await)
}
