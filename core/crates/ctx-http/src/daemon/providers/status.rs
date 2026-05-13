use std::sync::Arc;

use anyhow::Context;
use ctx_provider_install::install_state::InstallTarget;
use ctx_provider_runtime::provider_launch::status::provider_status_for_target;
use ctx_providers::adapters::ProviderStatus;

use crate::daemon::{execution_effective, AppState};

mod details;

use details::{decorate_provider_runtime_details, provider_status_without_target_bootstrap};

#[derive(Debug)]
pub(crate) enum ProviderStatusResponseError {
    NotFound { provider_id: String },
}

pub(crate) async fn install_target_for_workspace(
    state: &Arc<AppState>,
    workspace_id: ctx_core::ids::WorkspaceId,
) -> anyhow::Result<InstallTarget> {
    execution_effective::effective_install_target(state.as_ref(), workspace_id)
        .await
        .with_context(|| {
            format!(
                "loading execution settings for workspace {}",
                workspace_id.0
            )
        })
}

pub(crate) async fn providers_statuses_response(
    state: &Arc<AppState>,
    target: InstallTarget,
    include_matrix_providers: bool,
) -> Vec<ProviderStatus> {
    let (managed, managed_config_error) =
        ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    let matrix = state
        .providers
        .load_provider_matrix(&state.core.data_root)
        .await;
    let provider_ids = provider_status_ids(state, &matrix, include_matrix_providers).await;
    let mut out = Vec::with_capacity(provider_ids.len());
    for provider_id in provider_ids {
        let status = if managed_config_error.is_some() {
            provider_status_without_target_bootstrap(state, &provider_id, target).await
        } else {
            provider_status_for_target(state.as_ref(), &managed, &matrix, &provider_id, target)
                .await
        };
        out.push(status);
    }
    decorate_provider_statuses(state, &matrix, &managed_config_error, target, &mut out).await;
    out
}

pub(crate) async fn provider_status_response(
    state: &Arc<AppState>,
    provider_id: &str,
    target: InstallTarget,
) -> Result<ProviderStatus, ProviderStatusResponseError> {
    let (managed, managed_config_error) =
        ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    let matrix = state
        .providers
        .load_provider_matrix(&state.core.data_root)
        .await;
    ensure_known_provider(state, &matrix, provider_id).await?;

    let mut status = if managed_config_error.is_some() {
        provider_status_without_target_bootstrap(state, provider_id, target).await
    } else {
        provider_status_for_target(state.as_ref(), &managed, &matrix, provider_id, target).await
    };
    decorate_provider_runtime_details(
        state,
        &matrix,
        managed_config_error.as_deref(),
        target,
        &mut status,
    )
    .await;
    Ok(status)
}

async fn provider_status_ids(
    state: &Arc<AppState>,
    matrix: &ctx_provider_matrix::ProviderMatrix,
    include_matrix_providers: bool,
) -> Vec<String> {
    state
        .providers
        .visible_provider_status_ids(matrix, include_matrix_providers)
        .await
}

async fn ensure_known_provider(
    state: &Arc<AppState>,
    matrix: &ctx_provider_matrix::ProviderMatrix,
    provider_id: &str,
) -> Result<(), ProviderStatusResponseError> {
    if state
        .providers
        .is_known_provider_id(matrix, provider_id)
        .await
    {
        return Ok(());
    }

    Err(ProviderStatusResponseError::NotFound {
        provider_id: provider_id.to_string(),
    })
}

async fn decorate_provider_statuses(
    state: &Arc<AppState>,
    matrix: &ctx_provider_matrix::ProviderMatrix,
    managed_config_error: &Option<String>,
    target: InstallTarget,
    statuses: &mut [ProviderStatus],
) {
    let show_fake = std::env::var("CTX_SHOW_FAKE_PROVIDER")
        .ok()
        .as_deref()
        .and_then(ctx_core::boolish::parse_boolish)
        .unwrap_or(false);
    for status in statuses {
        details::decorate_provider_list_status(
            state,
            matrix,
            managed_config_error.as_deref(),
            target,
            show_fake,
            status,
        )
        .await;
    }
}
