use std::time::Duration;

use ctx_core::ids::WorkspaceId;
use ctx_core::models::Workspace;
use ctx_harness_sources::HarnessEndpointRecord;
use ctx_provider_install::install_state::InstallTarget;

use super::*;

pub(super) enum ProviderOptionsLoadOutcome {
    Cached(serde_json::Value),
    Ready(Box<ProviderOptionsInputs>),
}

pub(super) struct ProviderOptionsInputs {
    pub(super) workspace_id: WorkspaceId,
    pub(super) install_target: InstallTarget,
    pub(super) launch_config: ProviderLaunchConfigSnapshot,
    pub(super) cache: ProviderOptionsCacheSnapshot,
    pub(super) workspace: Workspace,
    pub(super) preferred_model_id: Option<String>,
    pub(super) selected_endpoint: Option<HarnessEndpointRecord>,
}

pub(super) async fn load_provider_options_inputs(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
    cache_ttl: Duration,
    verify_ttl: Duration,
) -> Result<ProviderOptionsLoadOutcome, ProviderOptionsResponseError> {
    let install_target = install_target_for_workspace(state, workspace_id)
        .await
        .map_err(ProviderOptionsResponseError::ExecutionSettings)?;
    let launch_config = load_provider_launch_config_snapshot(state, provider_id).await;
    let skip_cached_config_surfaces =
        launch_config.managed_config_error.is_some() || launch_config.source_config_error.is_some();
    let cache = ProviderOptionsCacheSnapshot::load(
        state,
        workspace_id,
        install_target,
        provider_id,
        skip_cached_config_surfaces,
    )
    .await;

    if let Some(out) = cache.fresh_authoritative_response(cache_ttl, verify_ttl) {
        return Ok(ProviderOptionsLoadOutcome::Cached(out));
    }
    launch_config
        .ensure_known_provider(state, provider_id)
        .await
        .map_err(ProviderOptionsResponseError::ProviderLaunchConfig)?;

    let workspace = load_workspace(state, workspace_id).await?;
    let preferred_model_id =
        load_workspace_preferred_model_id(state, workspace_id, provider_id).await?;
    let selected_endpoint = launch_config.selected_endpoint_record();

    Ok(ProviderOptionsLoadOutcome::Ready(Box::new(
        ProviderOptionsInputs {
            workspace_id,
            install_target,
            launch_config,
            cache,
            workspace,
            preferred_model_id,
            selected_endpoint,
        },
    )))
}

async fn load_workspace(
    state: &Arc<AppState>,
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
    state: &Arc<AppState>,
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
