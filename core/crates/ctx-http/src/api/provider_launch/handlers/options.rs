use super::*;

mod branches;
mod errors;
mod load;
mod probes;

use branches::{
    env_probe_provider_options, runtime_models_provider_options,
    selected_endpoint_runtime_launch_provider_options, ProviderOptionsProbeContext,
};
use errors::{
    auth_config_error_provider_options, managed_config_error_provider_options,
    source_config_error_provider_options, unusable_provider_options,
};
use load::{load_provider_options_inputs, ProviderOptionsInputs, ProviderOptionsLoadOutcome};
use probes::{
    probe_provider_options_env, probe_runtime_models_for_provider_options,
    probe_selected_endpoint_runtime_launch,
};

pub(in crate::api) async fn get_provider_options(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    const CACHE_TTL: Duration = Duration::from_secs(30);
    const VERIFY_TTL: Duration = Duration::from_secs(30 * 60);

    let inputs =
        match load_provider_options_inputs(&state, &ws_id, &provider_id, CACHE_TTL, VERIFY_TTL)
            .await?
        {
            ProviderOptionsLoadOutcome::Cached(out) => return Ok(Json(out)),
            ProviderOptionsLoadOutcome::Ready(inputs) => inputs,
        };
    let ProviderOptionsInputs {
        workspace_id: ws_id,
        install_target,
        managed,
        managed_config_error,
        matrix,
        source_config,
        source_config_error,
        cache,
        workspace,
        preferred_model_id,
        selected_endpoint,
    } = inputs;

    if let Some(config_error) = managed_config_error.as_ref() {
        let out = managed_config_error_provider_options(
            &state,
            &provider_id,
            ws_id,
            config_error,
            source_config.as_ref(),
            &cache,
            preferred_model_id.clone(),
            VERIFY_TTL,
        )
        .await;
        return Ok(Json(out));
    }

    let provider_status = provider_status_for_target(
        state.as_ref(),
        &managed,
        &matrix,
        &provider_id,
        install_target,
    )
    .await;

    if let Some(config_error) = source_config_error.as_ref() {
        let out = source_config_error_provider_options(
            &state,
            &provider_id,
            ws_id,
            &provider_status,
            config_error,
            source_config.as_ref(),
            &cache,
            preferred_model_id.clone(),
            VERIFY_TTL,
        )
        .await;
        return Ok(Json(out));
    }

    let has_active_auth = match probe::provider_has_active_auth_for_workspace_runtime(
        state.as_ref(),
        &workspace,
        &provider_id,
        source_config.as_ref(),
    )
    .await
    {
        Ok(value) => value,
        Err(config_error) => {
            let config_error = logs::redact_sensitive(&config_error);
            let out = auth_config_error_provider_options(
                &state,
                &provider_id,
                ws_id,
                &provider_status,
                &config_error,
                &cache,
                preferred_model_id.clone(),
                VERIFY_TTL,
            )
            .await;
            return Ok(Json(out));
        }
    };
    let auth_mode = provider_auth_mode(has_active_auth, source_config.as_ref());

    if !provider_status_is_usable(&provider_status) {
        let out = unusable_provider_options(
            &state,
            &provider_id,
            ws_id,
            &provider_status,
            has_active_auth,
            auth_mode,
            source_config.as_ref(),
            selected_endpoint.as_ref(),
            &cache,
            preferred_model_id.clone(),
            VERIFY_TTL,
        )
        .await;
        return Ok(Json(out));
    }

    let use_crp_probe = provider_supports_runtime_model_catalog(&provider_id);
    let probe_context = ProviderOptionsProbeContext {
        state: &state,
        workspace: &workspace,
        provider_id: &provider_id,
        workspace_id: ws_id,
        provider_status: &provider_status,
        has_active_auth,
        auth_mode,
        source_config: source_config.as_ref(),
        selected_endpoint: selected_endpoint.as_ref(),
        cache: &cache,
        preferred_model_id,
        verify_ttl: VERIFY_TTL,
    };

    match provider_options_probe_plan(
        use_crp_probe,
        selected_endpoint
            .as_ref()
            .map(|endpoint| endpoint.id.as_str()),
    ) {
        ProviderOptionsProbePlan::EnvOnly => {
            return env_probe_provider_options(probe_context).await;
        }
        ProviderOptionsProbePlan::SelectedEndpointRuntimeLaunch(endpoint_id) => {
            return selected_endpoint_runtime_launch_provider_options(
                probe_context,
                endpoint_id.to_string(),
            )
            .await;
        }
        ProviderOptionsProbePlan::RuntimeModels => {}
    }

    runtime_models_provider_options(probe_context).await
}
