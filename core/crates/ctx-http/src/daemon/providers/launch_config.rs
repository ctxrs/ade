use std::sync::Arc;

use ctx_harness_sources::HarnessEndpointRecord;
use ctx_provider_install::install_state::InstallTarget;
use ctx_provider_runtime::provider_auth::{
    selected_endpoint_from_harness_config, selected_endpoint_record_from_harness_config,
};
use ctx_provider_runtime::provider_launch::status::provider_status_for_target;
use ctx_providers::adapters::ProviderStatus;

use crate::daemon::providers::status::provider_status_without_target_bootstrap;
use crate::daemon::DaemonState;

pub(crate) struct ProviderLaunchConfigSnapshot {
    managed: ctx_managed_installs::AgentServerConfigFile,
    matrix: ctx_provider_matrix::ProviderMatrix,
    pub(crate) managed_config_error: Option<String>,
    pub(crate) source_config: Option<ctx_harness_sources::HarnessProviderSourceConfig>,
    pub(crate) source_config_error: Option<String>,
}

#[derive(Debug)]
pub(crate) enum ProviderLaunchConfigError {
    UnsupportedProvider { provider_id: String },
}

pub(crate) async fn load_provider_launch_config_snapshot(
    state: &Arc<DaemonState>,
    provider_id: &str,
) -> ProviderLaunchConfigSnapshot {
    let (managed, managed_config_error) =
        ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    let matrix = state
        .providers
        .load_provider_matrix(&state.core.data_root)
        .await;
    let (source_config, source_config_error) =
        ctx_provider_runtime::provider_launch::config::load_provider_source_config_with_error(
            &state.core.data_root,
            provider_id,
        )
        .await;

    ProviderLaunchConfigSnapshot {
        managed,
        matrix,
        managed_config_error,
        source_config,
        source_config_error,
    }
}

impl ProviderLaunchConfigSnapshot {
    pub(crate) async fn ensure_known_provider(
        &self,
        state: &Arc<DaemonState>,
        provider_id: &str,
    ) -> Result<(), ProviderLaunchConfigError> {
        if state
            .providers
            .is_known_provider_id(&self.matrix, provider_id)
            .await
        {
            return Ok(());
        }

        Err(ProviderLaunchConfigError::UnsupportedProvider {
            provider_id: provider_id.to_string(),
        })
    }

    pub(crate) async fn provider_status(
        &self,
        state: &Arc<DaemonState>,
        provider_id: &str,
        target: InstallTarget,
    ) -> ProviderStatus {
        if self.managed_config_error.is_some() {
            return provider_status_without_target_bootstrap(state, provider_id, target).await;
        }

        provider_status_for_target(
            state.as_ref(),
            &self.managed,
            &self.matrix,
            provider_id,
            target,
        )
        .await
    }

    pub(crate) fn selected_endpoint_record(&self) -> Option<HarnessEndpointRecord> {
        selected_endpoint_record_from_harness_config(self.source_config.as_ref())
    }

    pub(crate) fn selected_endpoint_id(&self) -> Option<String> {
        selected_endpoint_from_harness_config(self.source_config.clone())
    }

    pub(crate) fn source_config(
        &self,
    ) -> Option<&ctx_harness_sources::HarnessProviderSourceConfig> {
        self.source_config.as_ref()
    }
}
