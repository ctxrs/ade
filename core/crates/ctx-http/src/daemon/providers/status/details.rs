use std::sync::Arc;

use ctx_managed_installs as installer;
use ctx_provider_runtime::provider_launch::status::mark_provider_status_with_managed_config_error;
use ctx_providers::adapters::ProviderStatus;

use crate::daemon::DaemonState;

pub(crate) async fn provider_status_without_target_bootstrap(
    state: &Arc<DaemonState>,
    provider_id: &str,
    target: ctx_provider_install::install_state::InstallTarget,
) -> ProviderStatus {
    state
        .providers
        .provider_status_without_target_bootstrap(provider_id, target)
        .await
}

pub(super) async fn decorate_provider_list_status(
    state: &Arc<DaemonState>,
    matrix: &ctx_provider_matrix::ProviderMatrix,
    managed_config_error: Option<&str>,
    target: ctx_provider_install::install_state::InstallTarget,
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

pub(crate) async fn decorate_provider_runtime_details(
    state: &Arc<DaemonState>,
    matrix: &ctx_provider_matrix::ProviderMatrix,
    managed_config_error: Option<&str>,
    target: ctx_provider_install::install_state::InstallTarget,
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
