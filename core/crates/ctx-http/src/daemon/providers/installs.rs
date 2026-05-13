use std::sync::Arc;

use ctx_provider_install::install_state::{
    InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use tokio::sync::broadcast;

use crate::daemon::AppState;

pub(crate) use ctx_provider_runtime::provider_launch::install::StartProviderInstallError;

pub(crate) fn parse_provider_install_target(raw: Option<&str>) -> Result<InstallTarget, String> {
    ctx_managed_installs::parse_install_target(raw).map_err(|error| error.to_string())
}

pub(crate) async fn start_provider_install(
    state: &Arc<AppState>,
    provider_id: &str,
    target: InstallTarget,
) -> Result<InstallId, StartProviderInstallError> {
    let (install_id, _) = ctx_provider_runtime::provider_launch::install::start_provider_install(
        state,
        provider_id,
        target,
    )
    .await?;
    Ok(install_id)
}

pub(crate) async fn start_all_provider_installs(
    state: &Arc<AppState>,
    target: InstallTarget,
) -> Result<Vec<(String, InstallId)>, StartProviderInstallError> {
    ctx_provider_runtime::provider_launch::install::start_all_provider_installs(state, target).await
}

pub(crate) async fn get_provider_install_info(
    state: &Arc<AppState>,
    install_id: InstallId,
) -> Option<InstallInfo> {
    state.get_install_polling_info(install_id).await
}

pub(crate) async fn cancel_provider_install(
    state: &Arc<AppState>,
    install_id: InstallId,
) -> Option<InstallInfo> {
    state.cancel_install(install_id).await
}

pub(crate) async fn list_provider_install_events(
    state: &Arc<AppState>,
    install_id: InstallId,
) -> Option<Vec<InstallProgressEvent>> {
    state.get_install_events(install_id).await
}

pub(crate) async fn provider_install_event_sender(
    state: &Arc<AppState>,
    install_id: InstallId,
) -> Option<broadcast::Sender<InstallProgressEvent>> {
    state.get_install_sender(install_id).await
}
