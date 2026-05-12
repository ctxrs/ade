use super::*;

#[path = "aggregate/details.rs"]
mod details;

pub(in crate::api::providers::status) use details::{
    decorate_provider_runtime_details, provider_status_without_target_bootstrap,
};

pub(crate) async fn providers_statuses_response(
    state: &Arc<AppState>,
    target: InstallTarget,
    include_matrix_providers: bool,
) -> Vec<ProviderStatus> {
    let (managed, managed_config_error) =
        crate::api::provider_launch::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    let matrix = ctx_provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
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

async fn provider_status_ids(
    state: &Arc<AppState>,
    matrix: &ctx_provider_matrix::ProviderMatrix,
    include_matrix_providers: bool,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut provider_ids = Vec::new();
    state
        .providers
        .with_provider_statuses(|map| {
            for provider_id in map.keys() {
                if !ctx_provider_matrix::is_user_facing_harness_id(matrix, provider_id) {
                    continue;
                }
                if seen.insert(provider_id.clone()) {
                    provider_ids.push(provider_id.clone());
                }
            }
        })
        .await;
    if include_matrix_providers {
        for entry in &matrix.providers {
            if entry.kind != ctx_provider_matrix::ProviderMatrixEntryKind::Harness {
                continue;
            }
            if seen.insert(entry.id.clone()) {
                provider_ids.push(entry.id.clone());
            }
        }
    }
    provider_ids
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
