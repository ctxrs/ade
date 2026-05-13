use std::time::Duration;

use super::*;
use ctx_core::ids::WorkspaceId;
use ctx_harness_sources::HarnessEndpointRecord;
use ctx_providers::adapters::ProviderStatus;

pub(super) struct ProviderOptionsProbeContext<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) workspace: &'a ctx_core::models::Workspace,
    pub(super) provider_id: &'a str,
    pub(super) workspace_id: WorkspaceId,
    pub(super) provider_status: &'a ProviderStatus,
    pub(super) has_active_auth: bool,
    pub(super) auth_mode: &'a str,
    pub(super) source_config: Option<&'a harness_sources::HarnessProviderSourceConfig>,
    pub(super) selected_endpoint: Option<&'a HarnessEndpointRecord>,
    pub(super) cache: &'a ProviderOptionsCacheSnapshot,
    pub(super) preferred_model_id: Option<String>,
    pub(super) verify_ttl: Duration,
}

impl ProviderOptionsProbeContext<'_> {
    fn response_base(&self) -> ProviderOptionsResponseBase<'_> {
        ProviderOptionsResponseBase {
            provider_id: self.provider_id,
            workspace_id: self.workspace_id,
            provider_status: self.provider_status,
            has_active_auth: self.has_active_auth,
            auth_mode: self.auth_mode,
            source_config: self.source_config,
        }
    }

    fn response_context(&self) -> ProviderOptionsResponseContext<'_> {
        ProviderOptionsResponseContext {
            state: self.state,
            provider_id: self.provider_id,
            provider_status: Some(self.provider_status),
            selected_endpoint: self.selected_endpoint,
            cache: self.cache,
            preferred_model_id: self.preferred_model_id.clone(),
        }
    }
}

pub(super) async fn env_probe_provider_options(
    context: ProviderOptionsProbeContext<'_>,
) -> Result<Value, ProviderOptionsResponseError> {
    let (probe_ok, auth_required, probe_error) =
        probe_provider_options_env(context.state, context.workspace, context.provider_id).await;
    let raw_resp = env_probe_provider_options_response(
        context.response_base(),
        ProviderOptionsProbeResult {
            probe_ok,
            auth_required,
            probe_error,
        },
    );
    let out = finalize_provider_options_response(
        context.response_context(),
        raw_resp,
        true,
        context.verify_ttl,
    )
    .await;
    Ok(out)
}

pub(super) async fn selected_endpoint_runtime_launch_provider_options(
    context: ProviderOptionsProbeContext<'_>,
    endpoint_id: String,
) -> Result<Value, ProviderOptionsResponseError> {
    let endpoint = context
        .selected_endpoint
        .ok_or(ProviderOptionsResponseError::SelectedEndpointMissing)?;
    let (probe_ok, auth_required, probe_error) = probe_selected_endpoint_runtime_launch(
        context.state,
        context.workspace,
        context.provider_id,
        endpoint_id,
    )
    .await?;
    let raw_resp = selected_endpoint_runtime_launch_options_response(
        context.response_base(),
        endpoint,
        ProviderOptionsProbeResult {
            probe_ok,
            auth_required,
            probe_error,
        },
    );
    let out = finalize_provider_options_response(
        context.response_context(),
        raw_resp,
        true,
        context.verify_ttl,
    )
    .await;
    Ok(out)
}

pub(super) async fn runtime_models_provider_options(
    context: ProviderOptionsProbeContext<'_>,
) -> Result<Value, ProviderOptionsResponseError> {
    let probe = probe_runtime_models_for_provider_options(
        context.state,
        context.workspace,
        context.provider_id,
    )
    .await?;
    let raw_resp = runtime_models_provider_options_response(
        context.provider_id,
        context.workspace_id,
        context.provider_status,
        probe,
        context.has_active_auth,
        context.auth_mode,
        context.source_config,
    );
    let out = finalize_provider_options_response(
        context.response_context(),
        raw_resp,
        true,
        context.verify_ttl,
    )
    .await;
    Ok(out)
}
