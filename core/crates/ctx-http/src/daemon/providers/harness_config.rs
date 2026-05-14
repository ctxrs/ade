use std::sync::Arc;

use ctx_harness_sources as harness_sources;

use crate::daemon::DaemonState;

pub(crate) async fn get_provider_harness_config(
    state: &Arc<DaemonState>,
    provider_id: &str,
) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
    harness_sources::get_provider_source_config(&state.core.data_root, provider_id).await
}

pub(crate) async fn select_provider_harness_source(
    state: &Arc<DaemonState>,
    provider_id: &str,
    source_kind: harness_sources::HarnessSourceKind,
    endpoint_id: Option<String>,
) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
    let config = harness_sources::set_provider_source_selection(
        &state.core.data_root,
        provider_id,
        source_kind,
        endpoint_id,
    )
    .await?;
    super::restarts::invalidate_provider_runtime_state(state, provider_id).await;
    Ok(config)
}

pub(crate) async fn upsert_provider_harness_endpoint(
    state: &Arc<DaemonState>,
    provider_id: &str,
    endpoint: harness_sources::HarnessEndpointUpsert,
    manual_model_ids: Option<Vec<String>>,
) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
    let endpoint =
        harness_sources::upsert_provider_endpoint(&state.core.data_root, provider_id, endpoint)
            .await?;
    if let Some(manual_model_ids) = manual_model_ids {
        harness_sources::set_provider_endpoint_manual_models(
            &state.core.data_root,
            provider_id,
            &endpoint.id,
            manual_model_ids,
        )
        .await?;
    }
    harness_sources::refresh_provider_endpoint_model_catalog(
        &state.core.data_root,
        provider_id,
        &endpoint.id,
    )
    .await?;
    super::restarts::invalidate_provider_runtime_state(state, provider_id).await;
    get_provider_harness_config(state, provider_id).await
}

pub(crate) async fn refresh_provider_harness_endpoint_models(
    state: &Arc<DaemonState>,
    provider_id: &str,
    endpoint_id: &str,
) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
    refresh_provider_endpoint_model_catalog(state, provider_id, endpoint_id).await?;
    super::restarts::invalidate_provider_runtime_state(state, provider_id).await;
    get_provider_harness_config(state, provider_id).await
}

pub(crate) async fn set_provider_harness_endpoint_manual_models(
    state: &Arc<DaemonState>,
    provider_id: &str,
    endpoint_id: &str,
    model_ids: Vec<String>,
) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
    harness_sources::set_provider_endpoint_manual_models(
        &state.core.data_root,
        provider_id,
        endpoint_id,
        model_ids,
    )
    .await?;
    super::restarts::invalidate_provider_runtime_state(state, provider_id).await;
    get_provider_harness_config(state, provider_id).await
}

pub(crate) async fn delete_provider_harness_endpoint(
    state: &Arc<DaemonState>,
    provider_id: &str,
    endpoint_id: &str,
) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
    let config =
        harness_sources::delete_provider_endpoint(&state.core.data_root, provider_id, endpoint_id)
            .await?;
    super::restarts::invalidate_provider_runtime_state(state, provider_id).await;
    Ok(config)
}

pub(crate) async fn refresh_provider_endpoint_model_catalog(
    state: &Arc<DaemonState>,
    provider_id: &str,
    endpoint_id: &str,
) -> anyhow::Result<harness_sources::HarnessEndpointRecord> {
    harness_sources::refresh_provider_endpoint_model_catalog(
        &state.core.data_root,
        provider_id,
        endpoint_id,
    )
    .await
}

pub(crate) async fn mark_provider_endpoint_verification(
    state: &Arc<DaemonState>,
    provider_id: &str,
    endpoint_id: &str,
    status: harness_sources::HarnessEndpointVerificationStatus,
    error: Option<String>,
) -> anyhow::Result<()> {
    harness_sources::mark_endpoint_verification(
        &state.core.data_root,
        provider_id,
        endpoint_id,
        status,
        error,
    )
    .await
}
