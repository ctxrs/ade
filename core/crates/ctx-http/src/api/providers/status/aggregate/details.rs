use super::*;

use ctx_provider_runtime::provider_launch::status::mark_provider_status_with_managed_config_error;

pub(in crate::api::providers::status) async fn provider_status_without_target_bootstrap(
    state: &Arc<AppState>,
    provider_id: &str,
    target: InstallTarget,
) -> ProviderStatus {
    state
        .providers
        .provider_status_without_target_bootstrap(provider_id, target)
        .await
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
