use std::collections::HashMap;
use std::path::Path;
use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use ctx_observability::ops_events::{OpsEvent, OpsEvents};
use ctx_provider_install::install_state::{
    InstallErrorCode, InstallId, InstallInfo, InstallProgressEvent, InstallStateKind, InstallTarget,
};
use ctx_provider_runtime::{
    provider_install_tracker::ProviderInstallOpsEvent, provider_usage::ProviderUsageHost,
    ProviderRuntime, ProviderRuntimeHost,
};
use ctx_providers::adapters::{ProviderAdapter, ProviderStatus};
use tokio::sync::broadcast;

use super::{
    handle::ProviderWorkspaceLaunchRuntime, ProviderAdminHandle, ProviderBootstrapHandle,
    ProviderStatusHandle, ProviderUsageHandle,
};

fn current_ctx_version_for_provider_runtime() -> Option<String> {
    match ctx_update_service::current_build_identity(env!("CARGO_PKG_VERSION")) {
        Ok(identity) => Some(identity.exact_version.clone()),
        Err(err) => {
            tracing::error!("failed to load ctx build identity for provider runtime: {err:#}");
            None
        }
    }
}

fn emit_provider_install_ops_events(ops_events: &OpsEvents, events: Vec<ProviderInstallOpsEvent>) {
    for event in events {
        let mut ops_event = OpsEvent::new(event.level, event.name);
        ops_event.provider_id = Some(event.provider_id);
        let mut meta = serde_json::Map::new();
        meta.insert(
            "install_id".to_string(),
            serde_json::Value::String(event.install_id.to_string()),
        );
        if let Some(target) = event.target {
            meta.insert(
                "target".to_string(),
                serde_json::Value::String(target.as_str().to_string()),
            );
        }
        if let Some(state) = event.state {
            meta.insert(
                "state".to_string(),
                serde_json::Value::String(
                    match state {
                        InstallStateKind::Running => "running",
                        InstallStateKind::Succeeded => "succeeded",
                        InstallStateKind::Failed => "failed",
                        InstallStateKind::Cancelled => "cancelled",
                    }
                    .to_string(),
                ),
            );
        }
        if let Some(error) = event.error {
            meta.insert("error".to_string(), serde_json::Value::String(error));
        }
        if let Some(error_code) = event
            .error_code
            .and_then(|value| serde_json::to_value(value).ok())
        {
            meta.insert("error_code".to_string(), error_code);
        }
        if let Some(ok) = event.ok {
            meta.insert("ok".to_string(), serde_json::Value::Bool(ok));
        }
        ops_event.meta = Some(serde_json::Value::Object(meta));
        ops_events.emit(ops_event);
    }
}

impl ProviderRuntimeHost for ProviderStatusHandle {
    fn data_root(&self) -> &Path {
        self.data_root()
    }

    fn current_ctx_version(&self) -> Option<String> {
        current_ctx_version_for_provider_runtime()
    }

    fn provider_runtime(&self) -> &ProviderRuntime {
        self.providers()
    }

    fn publish_provider_install_ops_events(&self, events: Vec<ProviderInstallOpsEvent>) {
        emit_provider_install_ops_events(self.ops_events(), events);
    }
}

impl ProviderRuntimeHost for ProviderAdminHandle {
    fn data_root(&self) -> &Path {
        self.data_root()
    }

    fn current_ctx_version(&self) -> Option<String> {
        current_ctx_version_for_provider_runtime()
    }

    fn provider_runtime(&self) -> &ProviderRuntime {
        self.providers()
    }

    fn publish_provider_install_ops_events(&self, events: Vec<ProviderInstallOpsEvent>) {
        emit_provider_install_ops_events(self.ops_events(), events);
    }
}

impl ProviderRuntimeHost for ProviderBootstrapHandle {
    fn data_root(&self) -> &Path {
        self.data_root()
    }

    fn current_ctx_version(&self) -> Option<String> {
        current_ctx_version_for_provider_runtime()
    }

    fn provider_runtime(&self) -> &ProviderRuntime {
        self.providers()
    }

    fn publish_provider_install_ops_events(&self, events: Vec<ProviderInstallOpsEvent>) {
        emit_provider_install_ops_events(self.ops_events(), events);
    }
}

impl ProviderRuntimeHost for ProviderWorkspaceLaunchRuntime {
    fn data_root(&self) -> &Path {
        self.data_root()
    }

    fn current_ctx_version(&self) -> Option<String> {
        current_ctx_version_for_provider_runtime()
    }

    fn provider_runtime(&self) -> &ProviderRuntime {
        self.providers()
    }

    fn publish_provider_install_ops_events(&self, events: Vec<ProviderInstallOpsEvent>) {
        emit_provider_install_ops_events(self.ops_events(), events);
    }
}

#[async_trait]
impl ctx_managed_installs::ManagedInstallHost for ProviderAdminHandle {
    fn data_root(&self) -> &Path {
        self.data_root()
    }

    fn current_ctx_version(&self) -> Option<String> {
        current_ctx_version_for_provider_runtime()
    }

    async fn load_provider_matrix(&self) -> ctx_provider_matrix::ProviderMatrix {
        self.providers()
            .load_provider_matrix(self.data_root())
            .await
    }

    async fn invalidate_provider_matrix_cache(&self) {
        self.providers().invalidate_provider_matrix_cache().await;
    }

    async fn inspect_provider_adapters(&self) -> Vec<(String, Result<ProviderStatus, String>)> {
        self.providers().inspect_provider_adapters().await
    }

    async fn upsert_provider_adapter(
        &self,
        provider_id: String,
        adapter: Arc<dyn ProviderAdapter>,
    ) {
        self.providers()
            .upsert_provider_adapter(provider_id, adapter)
            .await;
    }

    async fn upsert_target_provider_adapter(
        &self,
        cache_key: String,
        adapter: Arc<dyn ProviderAdapter>,
    ) {
        self.providers()
            .upsert_target_provider_adapter(cache_key, adapter)
            .await;
    }

    async fn replace_provider_statuses(&self, statuses: HashMap<String, ProviderStatus>) {
        self.providers().replace_provider_statuses(statuses).await;
    }

    fn validate_install_target_allowed(&self, target: InstallTarget) -> Result<()> {
        ctx_settings_service::HostExecutionPolicy::current()?.validate_install_target(target)
    }

    async fn start_install(
        &self,
        provider_id: String,
        target: Option<InstallTarget>,
    ) -> (InstallId, bool) {
        let outcome = self.providers().start_install(provider_id, target).await;
        emit_provider_install_ops_events(self.ops_events(), outcome.ops_events);
        (outcome.install_id, outcome.started_new)
    }

    async fn get_install_info(&self, install_id: InstallId) -> Option<InstallInfo> {
        let outcome = self.providers().get_install_info(install_id).await;
        emit_provider_install_ops_events(self.ops_events(), outcome.ops_events);
        outcome.info
    }

    async fn register_install_progress_mirror(
        &self,
        source_install_id: InstallId,
        mirror_install_id: InstallId,
    ) -> bool {
        self.providers()
            .register_install_progress_mirror(source_install_id, mirror_install_id)
            .await
    }

    async fn set_install_progress_pct_override(&self, install_id: InstallId, pct: Option<u8>) {
        self.providers()
            .set_install_progress_pct_override(install_id, pct)
            .await;
    }

    async fn emit_install_event(&self, install_id: InstallId, event: InstallProgressEvent) {
        self.providers().emit_install_event(install_id, event).await;
    }

    async fn finish_install(
        &self,
        install_id: InstallId,
        success: bool,
        error: Option<String>,
        error_code: Option<InstallErrorCode>,
    ) {
        let Some(event) = self
            .providers()
            .finish_install(install_id, success, error, error_code)
            .await
        else {
            return;
        };
        emit_provider_install_ops_events(self.ops_events(), vec![event]);
    }

    async fn is_install_cancelled(&self, install_id: InstallId) -> bool {
        self.providers().is_install_cancelled(install_id).await
    }

    async fn update_install_start_event(
        &self,
        install_id: InstallId,
        provider_id: &str,
        target: Option<InstallTarget>,
        message: String,
        only_if_default: bool,
    ) {
        self.providers()
            .update_install_start_event(install_id, provider_id, target, message, only_if_default)
            .await;
    }

    async fn ensure_builder_ready(&self) -> Result<()> {
        ctx_harness_runtime::container_builder::ensure_builder_ready(self.data_root()).await
    }

    async fn run_builder_command(
        &self,
        cwd: &Path,
        env: &[(String, String)],
        argv: &[String],
        timeout_dur: Duration,
    ) -> Result<Output> {
        ctx_harness_runtime::container_builder::run_command(
            self.data_root(),
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

impl ProviderUsageHost for ProviderUsageHandle {
    fn data_root(&self) -> &Path {
        self.data_root()
    }

    fn provider_runtime(&self) -> &ProviderRuntime {
        self.providers()
    }

    fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx().subscribe()
    }
}
