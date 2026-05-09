use super::*;
use ctx_provider_runtime::provider_launch::options::{
    provider_options_cache_entry_is_authoritative, provider_supports_runtime_model_catalog,
};
use ctx_session_tools::model_resolution::{build_model_catalog, ModelCatalog};

mod endpoint;
mod runtime;

use endpoint::{load_endpoint_model_catalog, EndpointModelCatalog};
use runtime::load_runtime_model_catalog;

async fn load_pinned_subscription_model_catalog(
    state: &Arc<AppState>,
    provider_id: &str,
    install_target: ctx_provider_install::install_state::InstallTarget,
) -> Result<Option<ModelCatalog>, String> {
    let (managed, config_error) =
        crate::api::provider_launch::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    if let Some(config_error) = config_error {
        return Err(config_error);
    }
    let matrix = ctx_provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    let provider_status = crate::api::providers::provider_status_for_target(
        state.as_ref(),
        &managed,
        &matrix,
        provider_id,
        install_target,
    )
    .await;
    let models_value = provider_accounts::pinned_subscription_models_value(
        provider_id,
        provider_status.version.as_deref(),
    );
    Ok(models_value.and_then(|value| build_model_catalog(&value)))
}

fn provider_model_cache_key(
    workspace: &Workspace,
    provider_id: &str,
    install_target: ctx_provider_install::install_state::InstallTarget,
) -> String {
    format!(
        "{}/{}/{}",
        workspace.id.0,
        install_target.as_str(),
        provider_id
    )
}

async fn load_provider_model_catalog_for_install_target(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
    install_target: ctx_provider_install::install_state::InstallTarget,
) -> Result<Option<ModelCatalog>, String> {
    let cache_key = provider_model_cache_key(workspace, provider_id, install_target);
    if let Some(entry) = state
        .providers
        .options_cache
        .lock()
        .await
        .get(&cache_key)
        .filter(|entry| provider_options_cache_entry_is_authoritative(provider_id, &entry.value))
    {
        if let Some(models) = entry.value.get("models") {
            if let Some(catalog) = build_model_catalog(models) {
                return Ok(Some(catalog));
            }
        }
    }

    match load_endpoint_model_catalog(state, workspace, provider_id, cache_key.clone()).await? {
        EndpointModelCatalog::Loaded(catalog) => return Ok(catalog),
        EndpointModelCatalog::NotEndpointSource => {}
    }

    if !provider_supports_runtime_model_catalog(provider_id) {
        return load_pinned_subscription_model_catalog(state, provider_id, install_target).await;
    }

    let pinned_catalog =
        load_pinned_subscription_model_catalog(state, provider_id, install_target).await?;
    load_runtime_model_catalog(
        state,
        workspace,
        provider_id,
        install_target,
        cache_key,
        pinned_catalog,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn load_provider_model_catalog(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
) -> Result<Option<ModelCatalog>, String> {
    let install_target =
        crate::daemon::execution_effective::effective_install_target(state.as_ref(), workspace.id)
            .await
            .map_err(|err| {
                format!("workspace execution settings unavailable for provider options: {err:#}")
            })?;
    load_provider_model_catalog_for_install_target(state, workspace, provider_id, install_target)
        .await
}

pub(crate) async fn load_provider_model_catalog_for_execution_environment(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
    execution_environment: ctx_core::models::ExecutionEnvironment,
) -> Result<Option<ModelCatalog>, String> {
    let install_target =
        crate::daemon::execution_effective::effective_install_target_for_environment(
            state.as_ref(),
            workspace.id,
            execution_environment,
        )
        .await
        .map_err(|err| {
            format!("workspace execution settings unavailable for provider options: {err:#}")
        })?;
    load_provider_model_catalog_for_install_target(state, workspace, provider_id, install_target)
        .await
}
