use ctx_provider_runtime::provider_launch::status::mark_provider_status_with_managed_config_error;
use ctx_providers::adapters::{ProviderHealth, ProviderUsability};

use super::*;

pub(in crate::api::providers::status) async fn provider_status_without_target_bootstrap(
    state: &Arc<AppState>,
    provider_id: &str,
    target: InstallTarget,
) -> ProviderStatus {
    if matches!(target, InstallTarget::Host) {
        return state
            .providers
            .statuses
            .lock()
            .await
            .get(provider_id)
            .cloned()
            .unwrap_or_else(|| missing_provider_status(provider_id));
    }

    ProviderStatus {
        provider_id: provider_id.to_string(),
        installed: false,
        detected_path: None,
        version: None,
        capabilities: None,
        health: ProviderHealth::Missing,
        diagnostics: vec![format!(
            "provider not available for target '{}'",
            target.as_str()
        )],
        details: std::collections::HashMap::new(),
        usability: ProviderUsability::default(),
    }
}

fn missing_provider_status(provider_id: &str) -> ProviderStatus {
    ProviderStatus {
        provider_id: provider_id.to_string(),
        installed: false,
        detected_path: None,
        version: None,
        capabilities: None,
        health: ProviderHealth::Missing,
        diagnostics: vec![format!("provider not available: {provider_id}")],
        details: std::collections::HashMap::new(),
        usability: ProviderUsability::default(),
    }
}

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
    {
        let map = state.providers.statuses.lock().await;
        for provider_id in map.keys() {
            if !ctx_provider_matrix::is_user_facing_harness_id(matrix, provider_id) {
                continue;
            }
            if seen.insert(provider_id.clone()) {
                provider_ids.push(provider_id.clone());
            }
        }
    }
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
        decorate_provider_list_status(
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

async fn decorate_provider_list_status(
    state: &Arc<AppState>,
    matrix: &ctx_provider_matrix::ProviderMatrix,
    managed_config_error: Option<&str>,
    target: InstallTarget,
    show_fake: bool,
    status: &mut ProviderStatus,
) {
    if status.provider_id == "fake" {
        status.details.insert(
            "ui_hidden".into(),
            if show_fake { "false" } else { "true" }.into(),
        );
    }
    status
        .details
        .insert("install_target".into(), target.as_str().to_string());
    decorate_provider_runtime_details(state, matrix, managed_config_error, target, status).await;
}

pub(in crate::api::providers::status) async fn decorate_provider_runtime_details(
    state: &Arc<AppState>,
    matrix: &ctx_provider_matrix::ProviderMatrix,
    managed_config_error: Option<&str>,
    target: InstallTarget,
    status: &mut ProviderStatus,
) {
    if let Some(bytes) =
        installer::managed_install_download_size_bytes(matrix, &status.provider_id, target)
    {
        status
            .details
            .insert("install_download_size_bytes".into(), bytes.to_string());
    }
    if let Some(install_id) = state
        .find_running_install(&status.provider_id, Some(target))
        .await
    {
        status
            .details
            .insert("install_running".into(), "true".into());
        status
            .details
            .insert("install_id".into(), install_id.to_string());
    }
    if let Some(config_error) = managed_config_error {
        mark_provider_status_with_managed_config_error(status, config_error);
    }
}
