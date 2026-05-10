use super::*;

use ctx_provider_runtime::provider_launch::status::mark_provider_status_with_managed_config_error;
use ctx_providers::adapters::{ProviderHealth, ProviderUsability};

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

pub(super) async fn decorate_provider_list_status(
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
