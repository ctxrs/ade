use crate::daemon::DaemonState;
use ctx_provider_install::install_state::{InstallId, InstallTarget};

#[async_trait::async_trait]
impl ctx_provider_runtime::provider_launch::install::ProviderInstallHost for DaemonState {
    async fn find_running_install(
        &self,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Option<InstallId> {
        DaemonState::find_running_install(self, provider_id, target).await
    }
}
