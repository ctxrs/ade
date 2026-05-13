use super::*;

mod cache;
mod load;
mod outcome;
mod probes;

use crate::api::provider_launch::errors::provider_launch_config_error_response;
use cache::store_provider_verify_cache;
use load::load_verify_workspace;
use outcome::{config_error_response, ProviderVerifyOutcome};
use probes::{run_catalog_verified_runtime_probe, run_direct_runtime_probe};

pub(in crate::api) async fn verify_provider_for_workspace(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<ProviderAuthCheckResp>, (StatusCode, Json<serde_json::Value>)> {
    let ws_id = parse_workspace_id(&ws_id)?;
    let workspace = load_verify_workspace(&state, ws_id).await?;
    let install_target = install_target_for_workspace(&state, workspace.id)
        .await
        .map_err(|error| workspace_execution_settings_error_json(&error))?;
    let launch_config = load_provider_launch_config_snapshot(&state, &provider_id).await;
    launch_config
        .ensure_known_provider(&state, &provider_id)
        .await
        .map_err(provider_launch_config_error_response)?;
    let checked_at = Utc::now().to_rfc3339();
    let selected_endpoint = launch_config.selected_endpoint_record();
    let mut selected_endpoint_id: Option<String> = launch_config.selected_endpoint_id();

    if let Some(config_error) = launch_config.managed_config_error.as_ref() {
        return Ok(Json(config_error_response(
            &provider_id,
            ws_id,
            &checked_at,
            config_error.clone(),
        )));
    }

    if let Some(config_error) = launch_config.source_config_error.as_ref() {
        return Ok(Json(config_error_response(
            &provider_id,
            ws_id,
            &checked_at,
            config_error.clone(),
        )));
    }

    let provider_status = launch_config
        .provider_status(&state, &provider_id, install_target)
        .await;
    let mut outcome = ProviderVerifyOutcome::new(checked_at, selected_endpoint_id.take());

    if !provider_status_is_usable(&provider_status) {
        outcome.apply_unusable_provider(
            provider_status_unusable_reason(&provider_status)
                .unwrap_or_else(|| "provider not ready for use".to_string()),
        );
    } else if let Some(endpoint) = selected_endpoint
        .as_ref()
        .filter(|endpoint| endpoint_supports_model_catalog_verify(endpoint))
    {
        match crate::daemon::providers::refresh_provider_endpoint_model_catalog(
            &state,
            &provider_id,
            &endpoint.id,
        )
        .await
        {
            Ok(refreshed_endpoint) => {
                outcome.apply_endpoint_catalog_refresh(refreshed_endpoint);
            }
            Err(err) => {
                outcome.apply_classified_probe_error(logs::redact_sensitive(&err.to_string()));
            }
        }

        if outcome.is_ok() {
            run_catalog_verified_runtime_probe(&state, &workspace, &provider_id, &mut outcome)
                .await?;
        }
    } else {
        run_direct_runtime_probe(&state, &workspace, &provider_id, &mut outcome).await?;
    }

    if let Some(endpoint_id) = outcome.selected_endpoint_id() {
        let _ = crate::daemon::providers::mark_provider_endpoint_verification(
            &state,
            &provider_id,
            endpoint_id,
            outcome.endpoint_status(),
            outcome.message().cloned(),
        )
        .await;
    }

    let resp = outcome.into_response(&provider_id, ws_id);
    store_provider_verify_cache(&state, ws_id, install_target, &provider_id, &resp).await;

    Ok(Json(resp))
}
