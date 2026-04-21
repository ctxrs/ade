use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::http::StatusCode;

use crate::api::providers::provider_status_for_target;
use crate::api::sessions::{load_provider_model_catalog_for_execution_environment, ModelCatalog};
use crate::daemon::AppState;
use crate::execution_effective;
use crate::provider_matrix::ProviderMatrixEntryKind;
use ctx_core::models::{ExecutionEnvironment, Workspace};

use super::errors::{api_error, internal_api_error, ApiResult};

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
    .map_err(internal_api_error)?;
    let managed = crate::installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    let mut known_providers = {
        let statuses = state.providers.statuses.lock().await;
        statuses.keys().cloned().collect::<HashSet<_>>()
    };
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
                StatusCode::BAD_REQUEST,
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
        if !crate::provider_usability::provider_status_is_usable(&status) {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                format!(
                    "harness '{provider_id}' is not ready: {}",
                    crate::provider_usability::provider_status_unusable_reason(&status,)
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
                return Err(api_error(StatusCode::BAD_REQUEST, error));
            }
        }
    }

    Ok(model_catalogs)
}
