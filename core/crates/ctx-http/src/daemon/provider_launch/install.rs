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
