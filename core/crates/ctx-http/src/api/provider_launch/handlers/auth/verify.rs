use super::*;

mod cache;
mod load;
mod outcome;
mod probes;

use cache::store_provider_verify_cache;
use load::{ensure_known_provider, load_verify_workspace};
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
    let (managed, managed_config_error) =
        load_managed_agent_server_config_with_error(&state.core.data_root).await;
    let matrix = state
        .providers
        .load_provider_matrix(&state.core.data_root)
        .await;
    ensure_known_provider(&state, &matrix, &provider_id).await?;
    let checked_at = Utc::now().to_rfc3339();
    let (source_config, source_config_error) =
        load_provider_source_config_with_error(&state.core.data_root, &provider_id).await;
    let selected_endpoint = selected_endpoint_record_from_harness_config(source_config.as_ref());
    let mut selected_endpoint_id: Option<String> =
        selected_endpoint_from_harness_config(source_config);

    if let Some(config_error) = managed_config_error {
        return Ok(Json(config_error_response(
            &provider_id,
            ws_id,
            &checked_at,
            config_error,
        )));
    }

    if let Some(config_error) = source_config_error {
        return Ok(Json(config_error_response(
            &provider_id,
            ws_id,
            &checked_at,
            config_error,
        )));
    }

    let provider_status = provider_status_for_target(
        state.as_ref(),
        &managed,
        &matrix,
        &provider_id,
        install_target,
    )
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
        match harness_sources::refresh_provider_endpoint_model_catalog(
            &state.core.data_root,
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
        let _ = harness_sources::mark_endpoint_verification(
            &state.core.data_root,
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
