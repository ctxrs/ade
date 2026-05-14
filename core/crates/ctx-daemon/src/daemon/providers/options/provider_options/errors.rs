use std::time::Duration;

use ctx_core::ids::WorkspaceId;
use ctx_harness_sources::HarnessEndpointRecord;
use ctx_providers::adapters::ProviderStatus;

use super::*;

pub(super) struct ProviderOptionsErrorContext<'a> {
    pub(super) state: &'a Arc<DaemonState>,
    pub(super) provider_id: &'a str,
    pub(super) workspace_id: WorkspaceId,
    pub(super) cache: &'a ProviderOptionsCacheSnapshot,
    pub(super) preferred_model_id: Option<String>,
    pub(super) verify_ttl: Duration,
}

pub(super) async fn managed_config_error_provider_options(
    context: ProviderOptionsErrorContext<'_>,
    config_error: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> serde_json::Value {
    let raw_resp = config_error_provider_options_response(
        context.provider_id,
        context.workspace_id,
        None,
        provider_auth_mode(false, source_config),
        config_error,
        source_config,
    );
    finalize_provider_options_response(
        ProviderOptionsResponseContext {
            state: context.state,
            provider_id: context.provider_id,
            provider_status: None,
            selected_endpoint: None,
            cache: context.cache,
            preferred_model_id: context.preferred_model_id,
        },
        raw_resp,
        false,
        context.verify_ttl,
    )
    .await
}

pub(super) async fn source_config_error_provider_options(
    context: ProviderOptionsErrorContext<'_>,
    provider_status: &ProviderStatus,
    config_error: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> serde_json::Value {
    let raw_resp = config_error_provider_options_response(
        context.provider_id,
        context.workspace_id,
        Some(provider_status.installed),
        provider_auth_mode(false, source_config),
        config_error,
        None,
    );
    finalize_provider_options_response(
        ProviderOptionsResponseContext {
            state: context.state,
            provider_id: context.provider_id,
            provider_status: Some(provider_status),
            selected_endpoint: None,
            cache: context.cache,
            preferred_model_id: context.preferred_model_id,
        },
        raw_resp,
        false,
        context.verify_ttl,
    )
    .await
}

pub(super) async fn auth_config_error_provider_options(
    context: ProviderOptionsErrorContext<'_>,
    provider_status: &ProviderStatus,
    config_error: &str,
) -> serde_json::Value {
    let raw_resp = config_error_provider_options_response(
        context.provider_id,
        context.workspace_id,
        Some(provider_status.installed),
        "none",
        config_error,
        None,
    );
    finalize_provider_options_response(
        ProviderOptionsResponseContext {
            state: context.state,
            provider_id: context.provider_id,
            provider_status: Some(provider_status),
            selected_endpoint: None,
            cache: context.cache,
            preferred_model_id: context.preferred_model_id,
        },
        raw_resp,
        false,
        context.verify_ttl,
    )
    .await
}

pub(super) async fn unusable_provider_options(
    context: ProviderOptionsErrorContext<'_>,
    provider_status: &ProviderStatus,
    has_active_auth: bool,
    auth_mode: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
    selected_endpoint: Option<&HarnessEndpointRecord>,
) -> serde_json::Value {
    let raw_base_resp = unusable_provider_options_response(
        context.provider_id,
        context.workspace_id,
        provider_status,
        has_active_auth,
        auth_mode,
        source_config,
    );
    finalize_provider_options_response(
        ProviderOptionsResponseContext {
            state: context.state,
            provider_id: context.provider_id,
            provider_status: Some(provider_status),
            selected_endpoint,
            cache: context.cache,
            preferred_model_id: context.preferred_model_id,
        },
        raw_base_resp,
        true,
        context.verify_ttl,
    )
    .await
}
