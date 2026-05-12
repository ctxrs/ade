use std::path::Path;
use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use ctx_provider_install::install_state::{
    InstallErrorCode, InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};

use crate::daemon::AppState as HttpAppState;

#[async_trait]
impl ctx_managed_installs::ManagedInstallHost for HttpAppState {
    fn data_root(&self) -> &Path {
        &self.core.data_root
    }

    fn current_ctx_version(&self) -> Option<String> {
        match ctx_update_service::current_build_identity(env!("CARGO_PKG_VERSION")) {
            Ok(identity) => Some(identity.exact_version.clone()),
            Err(err) => {
                tracing::error!("failed to load ctx build identity for managed installs: {err:#}");
                None
            }
        }
    }

    async fn load_provider_matrix(&self) -> ctx_provider_matrix::ProviderMatrix {
        self.providers
            .load_provider_matrix(&self.core.data_root)
            .await
    }

    async fn invalidate_provider_matrix_cache(&self) {
        self.providers.invalidate_provider_matrix_cache().await;
    }

    async fn inspect_provider_adapters(&self) -> Vec<(String, Result<ProviderStatus, String>)> {
        self.providers.inspect_provider_adapters().await
    }

    async fn upsert_provider_adapter(
        &self,
        provider_id: String,
        adapter: Arc<dyn ProviderAdapter>,
    ) {
        self.providers
            .upsert_provider_adapter(provider_id, adapter)
            .await;
    }

    async fn upsert_target_provider_adapter(
        &self,
        cache_key: String,
        adapter: Arc<dyn ProviderAdapter>,
    ) {
        self.providers
            .upsert_target_provider_adapter(cache_key, adapter)
            .await;
    }

    async fn replace_provider_statuses(
        &self,
        statuses: std::collections::HashMap<String, ProviderStatus>,
    ) {
        self.providers.replace_provider_statuses(statuses).await;
    }

    fn validate_install_target_allowed(&self, target: InstallTarget) -> Result<()> {
        ctx_settings_service::HostExecutionPolicy::current()?.validate_install_target(target)
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
        self.providers
            .update_install_start_event(install_id, provider_id, target, message, only_if_default)
            .await;
    }

    async fn ensure_builder_ready(&self) -> Result<()> {
        ctx_harness_runtime::container_builder::ensure_builder_ready(&self.core.data_root).await
    }

    async fn run_builder_command(
        &self,
        cwd: &Path,
        env: &[(String, String)],
        argv: &[String],
        timeout_dur: Duration,
    ) -> Result<Output> {
        ctx_harness_runtime::container_builder::run_command(
            &self.core.data_root,
            cwd,
            env,
            argv,
            timeout_dur,
        )
        .await
    }

    fn is_acp_provider_id(&self, provider_id: &str) -> bool {
        ctx_provider_runtime::provider_launch::resolver::is_acp_provider_id(provider_id)
    }

    fn normalize_acp_provider_command(
        &self,
        data_root: &Path,
        provider_id: &str,
        cmd: ctx_managed_installs::AgentServerCommand,
    ) -> Result<ctx_managed_installs::AgentServerCommand> {
        ctx_provider_runtime::provider_launch::resolver::normalize_acp_provider_command(
            data_root,
            provider_id,
            cmd,
        )
    }

    fn acp_bridge_command(
        &self,
        bridge_cmd: &ctx_managed_installs::AgentServerCommand,
        acp_cmd: ctx_managed_installs::AgentServerCommand,
    ) -> ctx_managed_installs::AgentServerCommand {
        ctx_provider_runtime::provider_launch::resolver::acp_bridge_command(bridge_cmd, acp_cmd)
    }
}
