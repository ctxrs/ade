use std::time::Duration;

use ctx_core::ids::WorkspaceId;
use ctx_harness_sources::HarnessEndpointRecord;
use ctx_providers::adapters::ProviderStatus;

use super::*;

pub(super) async fn managed_config_error_provider_options(
    state: &Arc<AppState>,
    provider_id: &str,
    workspace_id: WorkspaceId,
    config_error: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
    cache: &ProviderOptionsCacheSnapshot,
    preferred_model_id: Option<String>,
    verify_ttl: Duration,
) -> serde_json::Value {
    let raw_resp = config_error_provider_options_response(
        provider_id,
        workspace_id,
        None,
        provider_auth_mode(false, source_config),
        config_error,
        source_config,
    );
    finalize_provider_options_response(
        ProviderOptionsResponseContext {
            state,
            provider_id,
            provider_status: None,
            selected_endpoint: None,
            cache,
            preferred_model_id,
        },
        raw_resp,
        false,
        verify_ttl,
    )
    .await
}

pub(super) async fn source_config_error_provider_options(
    state: &Arc<AppState>,
    provider_id: &str,
    workspace_id: WorkspaceId,
    provider_status: &ProviderStatus,
    config_error: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
    cache: &ProviderOptionsCacheSnapshot,
    preferred_model_id: Option<String>,
    verify_ttl: Duration,
) -> serde_json::Value {
    let raw_resp = config_error_provider_options_response(
        provider_id,
        workspace_id,
        Some(provider_status.installed),
        provider_auth_mode(false, source_config),
        config_error,
        None,
    );
    finalize_provider_options_response(
        ProviderOptionsResponseContext {
            state,
            provider_id,
            provider_status: Some(provider_status),
            selected_endpoint: None,
            cache,
            preferred_model_id,
        },
        raw_resp,
        false,
        verify_ttl,
    )
    .await
}

pub(super) async fn auth_config_error_provider_options(
    state: &Arc<AppState>,
    provider_id: &str,
    workspace_id: WorkspaceId,
    provider_status: &ProviderStatus,
    config_error: &str,
    cache: &ProviderOptionsCacheSnapshot,
    preferred_model_id: Option<String>,
    verify_ttl: Duration,
) -> serde_json::Value {
    let raw_resp = config_error_provider_options_response(
        provider_id,
        workspace_id,
        Some(provider_status.installed),
        "none",
        config_error,
        None,
    );
    finalize_provider_options_response(
        ProviderOptionsResponseContext {
            state,
            provider_id,
            provider_status: Some(provider_status),
            selected_endpoint: None,
            cache,
            preferred_model_id,
        },
        raw_resp,
        false,
        verify_ttl,
    )
    .await
}

pub(super) async fn unusable_provider_options(
    state: &Arc<AppState>,
    provider_id: &str,
    workspace_id: WorkspaceId,
    provider_status: &ProviderStatus,
    has_active_auth: bool,
    auth_mode: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
    selected_endpoint: Option<&HarnessEndpointRecord>,
    cache: &ProviderOptionsCacheSnapshot,
    preferred_model_id: Option<String>,
    verify_ttl: Duration,
) -> serde_json::Value {
    let raw_base_resp = unusable_provider_options_response(
        provider_id,
        workspace_id,
        provider_status,
        has_active_auth,
        auth_mode,
        source_config,
    );
    finalize_provider_options_response(
        ProviderOptionsResponseContext {
            state,
            provider_id,
            provider_status: Some(provider_status),
            selected_endpoint,
            cache,
            preferred_model_id,
        },
        raw_base_resp,
        true,
        verify_ttl,
    )
    .await
}
