use std::sync::Arc;

use anyhow::Result;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::Workspace;

use ctx_provider_runtime::provider_cache;
use ctx_workspace_config as workspace_config;

use crate::daemon::DaemonState;

#[derive(Debug)]
pub(crate) enum WorkspaceProviderModelPreferenceError {
    ProviderIdRequired,
    ProviderNotFound { provider_id: String },
    WorkspaceNotFound,
    StoreUnavailable(anyhow::Error),
    ExecutionSettings(anyhow::Error),
}

pub(crate) struct WorkspaceProviderModelPreference {
    pub(crate) provider_id: String,
    pub(crate) preferred_model_id: Option<String>,
}

pub(crate) async fn get_workspace_provider_model_preference(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
) -> Result<WorkspaceProviderModelPreference, WorkspaceProviderModelPreferenceError> {
    let workspace = load_workspace(state, workspace_id).await?;
    let provider_id = require_configurable_provider_id(state, provider_id).await?;
    let preferred_model_id =
        load_workspace_provider_preferred_model_id(state, workspace_id, &provider_id).await?;
    let preferred_model_id = crate::daemon::providers::effective_preferred_model_id_for_workspace(
        state,
        &workspace,
        &provider_id,
        preferred_model_id,
    )
    .await
    .map_err(|error| match error {
        crate::daemon::providers::EffectivePreferredModelError::ExecutionSettings(error) => {
            WorkspaceProviderModelPreferenceError::ExecutionSettings(error)
        }
    })?;
    Ok(WorkspaceProviderModelPreference {
        provider_id,
        preferred_model_id,
    })
}

pub(crate) async fn set_workspace_provider_model_preference(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
    preferred_model_id: Option<String>,
) -> Result<WorkspaceProviderModelPreference, WorkspaceProviderModelPreferenceError> {
    let workspace = load_workspace(state, workspace_id).await?;
    let provider_id = require_configurable_provider_id(state, provider_id).await?;
    update_workspace_provider_preferred_model_id(
        state,
        workspace_id,
        &provider_id,
        preferred_model_id,
    )
    .await
    .map_err(WorkspaceProviderModelPreferenceError::StoreUnavailable)?;
    let preferred_model_id =
        load_workspace_provider_preferred_model_id(state, workspace_id, &provider_id).await?;
    let preferred_model_id = crate::daemon::providers::effective_preferred_model_id_for_workspace(
        state,
        &workspace,
        &provider_id,
        preferred_model_id,
    )
    .await
    .map_err(|error| match error {
        crate::daemon::providers::EffectivePreferredModelError::ExecutionSettings(error) => {
            WorkspaceProviderModelPreferenceError::ExecutionSettings(error)
        }
    })?;
    Ok(WorkspaceProviderModelPreference {
        provider_id,
        preferred_model_id,
    })
}

pub(crate) async fn load_workspace_provider_preferred_model_id(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
) -> Result<Option<String>, WorkspaceProviderModelPreferenceError> {
    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(WorkspaceProviderModelPreferenceError::StoreUnavailable)?;
    workspace_config::load_preferred_new_session_model_id(&store, provider_id)
        .await
        .map_err(WorkspaceProviderModelPreferenceError::StoreUnavailable)
}

pub(crate) async fn update_workspace_provider_preferred_model_id(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
    preferred_model_id: Option<String>,
) -> Result<()> {
    let store = state.store_for_workspace(workspace_id).await?;
    workspace_config::update_preferred_new_session_model_id(
        &store,
        provider_id,
        preferred_model_id,
    )
    .await?;
    provider_cache::invalidate_workspace_provider_options_cache(
        &state.providers,
        workspace_id,
        provider_id,
    )
    .await;
    Ok(())
}

async fn load_workspace(
    state: &Arc<DaemonState>,
    workspace_id: WorkspaceId,
) -> Result<Workspace, WorkspaceProviderModelPreferenceError> {
    state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(WorkspaceProviderModelPreferenceError::StoreUnavailable)?
        .ok_or(WorkspaceProviderModelPreferenceError::WorkspaceNotFound)
}

async fn require_configurable_provider_id(
    state: &Arc<DaemonState>,
    provider_id: &str,
) -> Result<String, WorkspaceProviderModelPreferenceError> {
    let provider_id = provider_id.trim();
    if provider_id.is_empty() {
        return Err(WorkspaceProviderModelPreferenceError::ProviderIdRequired);
    }

    let matrix = state
        .providers
        .load_provider_matrix(&state.core.data_root)
        .await;
    if state
        .providers
        .is_configurable_provider_id(&matrix, provider_id)
        .await
    {
        return Ok(provider_id.to_string());
    }

    Err(WorkspaceProviderModelPreferenceError::ProviderNotFound {
        provider_id: provider_id.to_string(),
    })
}
