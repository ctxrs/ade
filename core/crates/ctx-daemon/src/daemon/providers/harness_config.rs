use std::sync::Arc;

use ctx_harness_sources as harness_sources;
use ctx_observability::logs;
use serde::Deserialize;

use crate::daemon::{DaemonState, ProvidersHandle};

pub type ProviderHarnessSourceConfig = harness_sources::HarnessProviderSourceConfig;

#[derive(Debug, Deserialize)]
pub struct UpsertProviderHarnessEndpointRouteRequest {
    #[serde(default)]
    pub endpoint_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_shape: Option<harness_sources::HarnessApiShape>,
    #[serde(default)]
    pub auth_type: Option<String>,
    #[serde(default)]
    pub model_override: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub service_account_json: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub manual_model_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct SetProviderHarnessEndpointManualModelsRouteRequest {
    #[serde(default)]
    pub model_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderHarnessEndpointRouteErrorKind {
    BadRequest,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHarnessEndpointRouteError {
    kind: ProviderHarnessEndpointRouteErrorKind,
    message: String,
}

impl ProviderHarnessEndpointRouteError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: ProviderHarnessEndpointRouteErrorKind::BadRequest,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: ProviderHarnessEndpointRouteErrorKind::NotFound,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ProviderHarnessEndpointRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

pub async fn get_provider_harness_config(
    state: &Arc<DaemonState>,
    provider_id: &str,
) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
    harness_sources::get_provider_source_config(&state.core.data_root, provider_id).await
}

pub async fn select_provider_harness_source(
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

pub async fn upsert_provider_harness_endpoint(
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

impl ProvidersHandle {
    pub async fn upsert_provider_harness_endpoint_for_route(
        &self,
        provider_id: &str,
        request: UpsertProviderHarnessEndpointRouteRequest,
    ) -> Result<ProviderHarnessSourceConfig, ProviderHarnessEndpointRouteError> {
        let endpoint = harness_sources::HarnessEndpointUpsert {
            endpoint_id: request.endpoint_id,
            name: request.name,
            base_url: request.base_url,
            api_shape: request.api_shape,
            auth_type: request.auth_type,
            model_override: request.model_override,
            api_key: request.api_key,
            service_account_json: request.service_account_json,
            project_id: request.project_id,
            location: request.location,
        };
        upsert_provider_harness_endpoint(
            &self.state,
            provider_id,
            endpoint,
            request.manual_model_ids,
        )
        .await
        .map_err(provider_harness_endpoint_bad_request_error)
    }

    pub async fn refresh_provider_harness_endpoint_models_for_route(
        &self,
        provider_id: &str,
        endpoint_id: &str,
    ) -> Result<ProviderHarnessSourceConfig, ProviderHarnessEndpointRouteError> {
        refresh_provider_harness_endpoint_models(&self.state, provider_id, endpoint_id)
            .await
            .map_err(provider_harness_endpoint_bad_request_error)
    }

    pub async fn set_provider_harness_endpoint_manual_models_for_route(
        &self,
        provider_id: &str,
        endpoint_id: &str,
        request: SetProviderHarnessEndpointManualModelsRouteRequest,
    ) -> Result<ProviderHarnessSourceConfig, ProviderHarnessEndpointRouteError> {
        set_provider_harness_endpoint_manual_models(
            &self.state,
            provider_id,
            endpoint_id,
            request.model_ids,
        )
        .await
        .map_err(provider_harness_endpoint_bad_request_error)
    }

    pub async fn delete_provider_harness_endpoint_for_route(
        &self,
        provider_id: &str,
        endpoint_id: &str,
    ) -> Result<ProviderHarnessSourceConfig, ProviderHarnessEndpointRouteError> {
        delete_provider_harness_endpoint(&self.state, provider_id, endpoint_id)
            .await
            .map_err(provider_harness_endpoint_delete_error)
    }
}

fn provider_harness_endpoint_bad_request_error(
    error: anyhow::Error,
) -> ProviderHarnessEndpointRouteError {
    ProviderHarnessEndpointRouteError::bad_request(logs::redact_sensitive(&error.to_string()))
}

fn provider_harness_endpoint_delete_error(
    error: anyhow::Error,
) -> ProviderHarnessEndpointRouteError {
    let message = logs::redact_sensitive(&error.to_string());
    if message.contains("unknown endpoint") {
        ProviderHarnessEndpointRouteError::not_found(message)
    } else {
        ProviderHarnessEndpointRouteError::bad_request(message)
    }
}

pub async fn refresh_provider_harness_endpoint_models(
    state: &Arc<DaemonState>,
    provider_id: &str,
    endpoint_id: &str,
) -> anyhow::Result<harness_sources::HarnessProviderSourceConfig> {
    refresh_provider_endpoint_model_catalog(state, provider_id, endpoint_id).await?;
    super::restarts::invalidate_provider_runtime_state(state, provider_id).await;
    get_provider_harness_config(state, provider_id).await
}

pub async fn set_provider_harness_endpoint_manual_models(
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

pub async fn delete_provider_harness_endpoint(
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

pub async fn refresh_provider_endpoint_model_catalog(
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

pub async fn mark_provider_endpoint_verification(
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
