use super::*;

pub(super) async fn probe_provider_options_env(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
) -> (bool, bool, Option<String>) {
    let status =
        crate::daemon::providers::probe_provider_options_env(state, workspace, provider_id).await;
    (status.probe_ok, status.auth_required, status.probe_error)
}

pub(super) async fn probe_selected_endpoint_runtime_launch(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
    endpoint_id: String,
) -> Result<(bool, bool, Option<String>), (StatusCode, Json<serde_json::Value>)> {
    let status = crate::daemon::providers::probe_selected_endpoint_runtime_launch(
        state,
        workspace,
        provider_id,
        endpoint_id,
    )
    .await
    .map_err(|err| workspace_execution_settings_error_json(&err))?;
    Ok((status.probe_ok, status.auth_required, status.probe_error))
}

pub(super) async fn probe_runtime_models_for_provider_options(
    state: &Arc<AppState>,
    workspace: &ctx_core::models::Workspace,
    provider_id: &str,
) -> Result<anyhow::Result<ctx_providers::crp::CrpModelsProbe>, (StatusCode, Json<serde_json::Value>)>
{
    crate::daemon::providers::probe_runtime_models_for_provider_options(
        state,
        workspace,
        provider_id,
    )
    .await
    .map_err(|err| workspace_execution_settings_error_json(&err))
}
