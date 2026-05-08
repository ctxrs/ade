use std::sync::Arc;

use crate::daemon::AppState;
use ctx_provider_install::install_state::{InstallId, InstallTarget};

#[async_trait::async_trait]
impl ctx_provider_runtime::provider_launch::install::ProviderInstallHost for AppState {
    async fn find_running_install(
        &self,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Option<InstallId> {
        AppState::find_running_install(self, provider_id, target).await
    }
}

#[allow(unused_imports)]
pub(crate) use ctx_provider_runtime::provider_launch::install::{
    should_skip_install_for_healthy_provider,
    start_all_provider_installs as runtime_start_all_provider_installs, StartProviderInstallError,
};

pub(crate) async fn start_provider_install(
    state: &Arc<AppState>,
    provider_id: &str,
    target: InstallTarget,
) -> Result<(InstallId, bool), StartProviderInstallError> {
    ctx_provider_runtime::provider_launch::install::start_provider_install(
        state,
        provider_id,
        target,
    )
    .await
}

pub(crate) async fn start_all_provider_installs(
    state: &Arc<AppState>,
    target: InstallTarget,
) -> Result<Vec<(String, InstallId)>, StartProviderInstallError> {
    runtime_start_all_provider_installs(state, target).await
}
