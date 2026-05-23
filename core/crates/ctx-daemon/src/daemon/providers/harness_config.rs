use std::sync::Arc;

use ctx_harness_sources as harness_sources;
use ctx_provider_runtime::provider_harness_config as harness_config_service;

use crate::daemon::DaemonState;

pub mod routes;

pub async fn refresh_provider_endpoint_model_catalog(
    state: &Arc<DaemonState>,
    provider_id: &str,
    endpoint_id: &str,
) -> anyhow::Result<harness_sources::HarnessEndpointRecord> {
    harness_config_service::refresh_provider_endpoint_model_catalog(
        &state.core.data_root,
        provider_id,
        endpoint_id,
    )
    .await
}

pub async fn mark_provider_endpoint_verification(
    state: &Arc<DaemonState>,
    provider_id: &str,
    endpoint_id: &str,
    status: harness_sources::HarnessEndpointVerificationStatus,
    error: Option<String>,
) -> anyhow::Result<()> {
    harness_config_service::mark_provider_endpoint_verification(
        &state.core.data_root,
        provider_id,
        endpoint_id,
        status,
        error,
    )
    .await
}
