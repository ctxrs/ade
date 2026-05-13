use std::sync::Arc;

use ctx_provider_runtime::provider_launch::probe;
use ctx_provider_runtime::provider_launch::probe_error::classify_probe_error;
use ctx_provider_runtime::provider_launch::runtime_probe::{
    prepare_provider_runtime_probe_launch, PreparedProviderRuntimeProbe,
};
use ctx_providers::crp::{probe_crp_models, CrpModelsProbe};

use crate::daemon::providers::install_target_for_workspace;
use crate::daemon::AppState;

pub(crate) enum PreparedProviderRuntimeProbeError {
    ExecutionSettings(anyhow::Error),
    Verify(String),
}

pub(crate) struct ProviderRuntimeProbeStatus {
    pub(crate) probe_ok: bool,
    pub(crate) auth_required: bool,
    pub(crate) probe_error: Option<String>,
}

pub(crate) async fn prepare_provider_runtime_probe(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
    selected_endpoint_id: Option<String>,
) -> Result<PreparedProviderRuntimeProbe, PreparedProviderRuntimeProbeError> {
    let install_target = install_target_for_workspace(state, workspace.id)
        .await
        .map_err(PreparedProviderRuntimeProbeError::ExecutionSettings)?;
    let (cfg, config_error) =
        ctx_provider_runtime::provider_launch::config::load_managed_agent_server_config_with_error(
            &state.core.data_root,
        )
        .await;
    if let Some(config_error) = config_error {
        return Err(PreparedProviderRuntimeProbeError::Verify(config_error));
    }
    let probe_context =
        probe::provider_probe_context_for_workspace_runtime(state.as_ref(), workspace, provider_id)
            .await
            .map_err(PreparedProviderRuntimeProbeError::Verify)?;
    prepare_provider_runtime_probe_launch(
        &state.core.data_root,
        &cfg,
        provider_id,
        install_target,
        probe_context,
        selected_endpoint_id,
    )
    .map_err(|error| PreparedProviderRuntimeProbeError::Verify(error.into_message()))
}

pub(crate) async fn provider_has_active_auth_for_workspace_runtime(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
    source_config: Option<&ctx_harness_sources::HarnessProviderSourceConfig>,
) -> Result<bool, String> {
    probe::provider_has_active_auth_for_workspace_runtime(
        state.as_ref(),
        workspace,
        provider_id,
        source_config,
    )
    .await
}

pub(crate) async fn probe_provider_options_env(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
) -> ProviderRuntimeProbeStatus {
    match probe::provider_probe_env_for_workspace_runtime(state.as_ref(), workspace, provider_id)
        .await
    {
        Ok(_) => ProviderRuntimeProbeStatus {
            probe_ok: true,
            auth_required: false,
            probe_error: None,
        },
        Err(err) => classified_probe_status(ctx_observability::logs::redact_sensitive(&err)),
    }
}

pub(crate) async fn probe_selected_endpoint_runtime_launch(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
    endpoint_id: String,
) -> Result<ProviderRuntimeProbeStatus, anyhow::Error> {
    match prepare_provider_runtime_probe(state, workspace, provider_id, Some(endpoint_id)).await {
        Ok(prepared) => {
            match probe_crp_models(
                provider_id,
                prepared.command,
                prepared.args,
                prepared.cwd,
                prepared.env,
            )
            .await
            {
                Ok(_) => Ok(ProviderRuntimeProbeStatus {
                    probe_ok: true,
                    auth_required: false,
                    probe_error: None,
                }),
                Err(err) => Ok(classified_probe_status(
                    ctx_observability::logs::redact_sensitive(&err.to_string()),
                )),
            }
        }
        Err(PreparedProviderRuntimeProbeError::ExecutionSettings(err)) => Err(err),
        Err(PreparedProviderRuntimeProbeError::Verify(err)) => Ok(classified_probe_status(
            ctx_observability::logs::redact_sensitive(&err),
        )),
    }
}

pub(crate) async fn probe_runtime_models_for_provider_options(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
) -> Result<anyhow::Result<CrpModelsProbe>, anyhow::Error> {
    match prepare_provider_runtime_probe(state, workspace, provider_id, None).await {
        Ok(prepared) => Ok(probe_crp_models(
            provider_id,
            prepared.command,
            prepared.args,
            prepared.cwd,
            prepared.env,
        )
        .await),
        Err(PreparedProviderRuntimeProbeError::ExecutionSettings(err)) => Err(err),
        Err(PreparedProviderRuntimeProbeError::Verify(err)) => Ok(Err(anyhow::anyhow!(err))),
    }
}

fn classified_probe_status(probe_error: String) -> ProviderRuntimeProbeStatus {
    let (_, auth_required, _) = classify_probe_error(&probe_error);
    ProviderRuntimeProbeStatus {
        probe_ok: false,
        auth_required: auth_required.unwrap_or(false),
        probe_error: Some(probe_error),
    }
}
