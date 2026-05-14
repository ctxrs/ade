use std::sync::Arc;

use ctx_provider_install::install_state::{
    InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use tokio::sync::broadcast;

use crate::daemon::DaemonState;

pub use ctx_provider_runtime::provider_launch::install::StartProviderInstallError;

pub fn parse_provider_install_target(raw: Option<&str>) -> Result<InstallTarget, String> {
    ctx_managed_installs::parse_install_target(raw).map_err(|error| error.to_string())
}

pub async fn start_provider_install(
    state: &Arc<DaemonState>,
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

pub async fn start_all_provider_installs(
    state: &Arc<DaemonState>,
    target: InstallTarget,
) -> Result<Vec<(String, InstallId)>, StartProviderInstallError> {
    ctx_provider_runtime::provider_launch::install::start_all_provider_installs(state, target).await
}

pub async fn get_provider_install_info(
    state: &Arc<DaemonState>,
    install_id: InstallId,
) -> Option<InstallInfo> {
    state.get_install_polling_info(install_id).await
}

pub async fn cancel_provider_install(
    state: &Arc<DaemonState>,
    install_id: InstallId,
) -> Option<InstallInfo> {
    state.cancel_install(install_id).await
}

pub async fn list_provider_install_events(
    state: &Arc<DaemonState>,
    install_id: InstallId,
) -> Option<Vec<InstallProgressEvent>> {
    state.get_install_events(install_id).await
}

pub async fn provider_install_event_sender(
    state: &Arc<DaemonState>,
    install_id: InstallId,
) -> Option<broadcast::Sender<InstallProgressEvent>> {
    state.get_install_sender(install_id).await
}
