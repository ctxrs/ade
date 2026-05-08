use super::*;

pub(super) async fn probe_provider_options_env(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
) -> (bool, bool, Option<String>) {
    match probe::provider_probe_env_for_workspace_runtime(state.as_ref(), workspace, provider_id)
        .await
    {
        Ok(_) => (true, false, None),
        Err(err) => {
            let probe_error = logs::redact_sensitive(&err);
            let (_, auth_required, _) = classify_probe_error(&probe_error);
            (false, auth_required.unwrap_or(false), Some(probe_error))
        }
    }
}

pub(super) async fn probe_selected_endpoint_runtime_launch(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
    endpoint_id: String,
) -> Result<(bool, bool, Option<String>), (StatusCode, Json<serde_json::Value>)> {
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
                Ok(_) => Ok((true, false, None)),
                Err(err) => {
                    let probe_error = logs::redact_sensitive(&err.to_string());
                    let (_, auth_required, _) = classify_probe_error(&probe_error);
                    Ok((false, auth_required.unwrap_or(false), Some(probe_error)))
                }
            }
        }
        Err(PreparedProviderRuntimeProbeError::Route(err)) => Err(err),
        Err(PreparedProviderRuntimeProbeError::Verify(err)) => {
            let probe_error = logs::redact_sensitive(&err);
            let (_, auth_required, _) = classify_probe_error(&probe_error);
            Ok((false, auth_required.unwrap_or(false), Some(probe_error)))
        }
    }
}

pub(super) async fn probe_runtime_models_for_provider_options(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
) -> Result<anyhow::Result<ctx_providers::crp::CrpModelsProbe>, (StatusCode, Json<serde_json::Value>)>
{
    match prepare_provider_runtime_probe(state, workspace, provider_id, None).await {
        Ok(prepared) => Ok(probe_crp_models(
            provider_id,
            prepared.command,
            prepared.args,
            prepared.cwd,
            prepared.env,
        )
        .await),
        Err(PreparedProviderRuntimeProbeError::Route(err)) => Err(err),
        Err(PreparedProviderRuntimeProbeError::Verify(err)) => Ok(Err(anyhow::anyhow!(err))),
    }
}
