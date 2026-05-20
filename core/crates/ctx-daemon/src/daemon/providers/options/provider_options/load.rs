use std::sync::Arc;

use ctx_core::ids::WorkspaceId;
use ctx_core::models::Workspace;

use crate::daemon::DaemonState;

use super::ProviderOptionsResponseError;

pub(super) struct ProviderOptionsWorkspaceInputs {
    pub(super) workspace: Workspace,
    pub(super) preferred_model_id: Option<String>,
}

pub(super) async fn load_provider_options_workspace_inputs(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
) -> Result<ProviderOptionsWorkspaceInputs, ProviderOptionsResponseError> {
    let workspace = load_workspace(state, workspace_id).await?;
    let preferred_model_id =
        load_workspace_preferred_model_id(state, workspace_id, provider_id).await?;
    Ok(ProviderOptionsWorkspaceInputs {
        workspace,
        preferred_model_id,
    })
}

async fn load_workspace(
    state: &Arc<DaemonState>,
    ws_id: WorkspaceId,
) -> Result<Workspace, ProviderOptionsResponseError> {
    state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|_| ProviderOptionsResponseError::WorkspaceLoad)?
        .ok_or(ProviderOptionsResponseError::WorkspaceNotFound)
}

async fn load_workspace_preferred_model_id(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
) -> Result<Option<String>, ProviderOptionsResponseError> {
    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(ProviderOptionsResponseError::WorkspaceStoreLoad)?;
    ctx_workspace_config::load_preferred_new_session_model_id(&store, provider_id)
        .await
        .map_err(ProviderOptionsResponseError::WorkspacePreferenceLoad)
}
