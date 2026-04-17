use std::collections::HashMap;
use std::path::Path;
use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;

use ctx_provider_install::install_state::{
    InstallErrorCode, InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};

use crate::container_builder;
use crate::daemon::{self, AppState as HttpAppState};

pub use ctx_managed_installs::*;

#[async_trait]
impl ctx_managed_installs::ManagedInstallHost for HttpAppState {
    fn data_root(&self) -> &Path {
        &self.core.data_root
    }

    fn provider_matrix_cache(&self) -> &Mutex<ctx_provider_matrix::ProviderMatrixCache> {
        &self.providers.matrix_cache
    }

    fn provider_adapters(&self) -> &Mutex<HashMap<String, Arc<dyn ProviderAdapter>>> {
        &self.providers.adapters
    }

    fn target_provider_adapters(&self) -> &Mutex<HashMap<String, Arc<dyn ProviderAdapter>>> {
        &self.providers.target_adapters
    }

    fn provider_statuses(&self) -> &Mutex<HashMap<String, ProviderStatus>> {
        &self.providers.statuses
    }

    async fn start_install(
        &self,
        provider_id: String,
        target: Option<InstallTarget>,
    ) -> (InstallId, bool) {
        HttpAppState::start_install(self, provider_id, target).await
    }

    async fn get_install_info(&self, install_id: InstallId) -> Option<InstallInfo> {
        HttpAppState::get_install_info(self, install_id).await
    }

    async fn register_install_progress_mirror(
        &self,
        source_install_id: InstallId,
        mirror_install_id: InstallId,
    ) -> bool {
        HttpAppState::register_install_progress_mirror(self, source_install_id, mirror_install_id)
            .await
    }

    async fn set_install_progress_pct_override(&self, install_id: InstallId, pct: Option<u8>) {
        HttpAppState::set_install_progress_pct_override(self, install_id, pct).await;
    }

    async fn emit_install_event(&self, install_id: InstallId, event: InstallProgressEvent) {
        HttpAppState::emit_install_event(self, install_id, event).await;
    }

    async fn finish_install(
        &self,
        install_id: InstallId,
        success: bool,
        error: Option<String>,
        error_code: Option<InstallErrorCode>,
    ) {
        HttpAppState::finish_install(self, install_id, success, error, error_code).await;
    }

    async fn is_install_cancelled(&self, install_id: InstallId) -> bool {
        HttpAppState::is_install_cancelled(self, install_id).await
    }

    async fn update_install_start_event(
        &self,
        install_id: InstallId,
        provider_id: &str,
        target: Option<InstallTarget>,
        message: String,
        only_if_default: bool,
    ) {
        let mut installs = self.providers.installs.lock().await;
        let Some(install) = installs.get_mut(&install_id) else {
            return;
        };
        if only_if_default && !install.canonical_start_event_is_default() {
            return;
        }
        let _ = install.update_canonical_start_event(provider_id, target, message);
    }

    async fn ensure_builder_ready(&self) -> Result<()> {
        container_builder::ensure_builder_ready(&self.core.data_root).await
    }

    async fn run_builder_command(
        &self,
        cwd: &Path,
        env: &[(String, String)],
        argv: &[String],
        timeout_dur: Duration,
    ) -> Result<Output> {
        container_builder::run_command(&self.core.data_root, cwd, env, argv, timeout_dur).await
    }

    fn is_acp_provider_id(&self, provider_id: &str) -> bool {
        daemon::is_acp_provider_id(provider_id)
    }

    fn normalize_acp_provider_command(
        &self,
        data_root: &Path,
        provider_id: &str,
        cmd: ctx_managed_installs::AgentServerCommand,
    ) -> Result<ctx_managed_installs::AgentServerCommand> {
        daemon::normalize_acp_provider_command(data_root, provider_id, cmd)
    }

    fn acp_bridge_command(
        &self,
        bridge_cmd: &ctx_managed_installs::AgentServerCommand,
        acp_cmd: ctx_managed_installs::AgentServerCommand,
    ) -> ctx_managed_installs::AgentServerCommand {
        daemon::acp_bridge_command(bridge_cmd, acp_cmd)
    }
}
