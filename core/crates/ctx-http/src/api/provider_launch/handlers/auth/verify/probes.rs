use ctx_core::models::Workspace;

use super::*;
use crate::api::provider_launch::handlers::auth::verify::outcome::ProviderVerifyOutcome;

pub(super) async fn run_catalog_verified_runtime_probe(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
    outcome: &mut ProviderVerifyOutcome,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    match prepare_provider_runtime_probe(
        state,
        workspace,
        provider_id,
        outcome.selected_endpoint_id().map(str::to_string),
    )
    .await
    {
        Ok(prepared) => {
            outcome.set_selected_endpoint_id(prepared.selected_endpoint_id);
            if let Err(err) = probe_crp_models(
                provider_id,
                prepared.command,
                prepared.args,
                prepared.cwd,
                prepared.env,
            )
            .await
            {
                outcome.apply_endpoint_catalog_runtime_probe_failure(logs::redact_sensitive(
                    &err.to_string(),
                ));
            }
        }
        Err(PreparedProviderRuntimeProbeError::Route(err)) => return Err(err),
        Err(PreparedProviderRuntimeProbeError::Verify(err)) => {
            outcome.apply_endpoint_catalog_runtime_probe_failure(logs::redact_sensitive(&err));
        }
    }

    Ok(())
}

pub(super) async fn run_direct_runtime_probe(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
    outcome: &mut ProviderVerifyOutcome,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    match prepare_provider_runtime_probe(
        state,
        workspace,
        provider_id,
        outcome.selected_endpoint_id().map(str::to_string),
    )
    .await
    {
        Ok(prepared) => {
            outcome.set_selected_endpoint_id(prepared.selected_endpoint_id);
            let probe = probe_crp_models(
                provider_id,
                prepared.command,
                prepared.args,
                prepared.cwd,
                prepared.env,
            )
            .await;
            if let Err(err) = probe {
                outcome.apply_classified_probe_error(logs::redact_sensitive(&err.to_string()));
            }
        }
        Err(PreparedProviderRuntimeProbeError::Route(err)) => return Err(err),
        Err(PreparedProviderRuntimeProbeError::Verify(err)) => {
            outcome.apply_classified_probe_error(logs::redact_sensitive(&err));
        }
    }

    Ok(())
}
