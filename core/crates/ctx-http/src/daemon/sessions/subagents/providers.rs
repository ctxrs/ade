use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::api::providers::provider_status_for_target;
use crate::api::sessions::load_provider_model_catalog_for_execution_environment;
use crate::daemon::execution_effective;
use crate::daemon::AppState;
use ctx_core::models::{ExecutionEnvironment, Workspace};
use ctx_provider_matrix::ProviderMatrixEntryKind;
use ctx_provider_runtime::provider_usability::{
    provider_status_is_usable, provider_status_unusable_reason,
};
use ctx_session_tools::model_resolution::ModelCatalog;

use super::errors::{
    api_error, internal_api_error, internal_request_or_policy_error, ApiResult, SubagentErrorKind,
};

pub(super) async fn load_requested_model_catalogs(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_ids: &HashSet<String>,
    execution_environment: ExecutionEnvironment,
) -> ApiResult<HashMap<String, Option<ModelCatalog>>> {
    let install_target = execution_effective::effective_install_target_for_environment(
        state.as_ref(),
        workspace.id,
        execution_environment,
    )
    .await
    .map_err(internal_request_or_policy_error)?;
    let managed =
        crate::daemon::installer::load_managed_agent_server_config_or_err(&state.core.data_root)
            .await
            .map_err(internal_api_error)?;
    let matrix = state
        .providers
        .load_provider_matrix(&state.core.data_root)
        .await;
    let known_providers = state.providers.provider_status_ids().await;
    let mut known_providers = known_providers.into_iter().collect::<HashSet<_>>();
    for entry in &matrix.providers {
        if entry.kind == ProviderMatrixEntryKind::Harness {
            known_providers.insert(entry.id.clone());
        }
    }

    let mut available_providers = known_providers.iter().cloned().collect::<Vec<_>>();
    available_providers.sort();

    for provider_id in provider_ids {
        if !known_providers.contains(provider_id) {
            return Err(api_error(
                SubagentErrorKind::BadRequest,
                format!(
                    "unknown harness '{provider_id}'; available harnesses: {}",
                    available_providers.join(", ")
                ),
            ));
        }

        let status = provider_status_for_target(
            state.as_ref(),
            &managed,
            &matrix,
            provider_id,
            install_target,
        )
        .await;
        if !provider_status_is_usable(&status) {
            return Err(api_error(
                SubagentErrorKind::BadRequest,
                format!(
                    "harness '{provider_id}' is not ready: {}",
                    provider_status_unusable_reason(&status)
                        .unwrap_or_else(|| "provider not ready for use".to_string())
                ),
            ));
        }
    }

    let mut model_catalogs = HashMap::new();
    for provider_id in provider_ids {
        let catalog = load_provider_model_catalog_for_execution_environment(
            state,
            workspace,
            provider_id,
            execution_environment,
        )
        .await;
        match catalog {
            Ok(catalog) => {
                model_catalogs.insert(provider_id.clone(), catalog);
            }
            Err(error) => {
                return Err(api_error(SubagentErrorKind::BadRequest, error));
            }
        }
    }

    Ok(model_catalogs)
}
