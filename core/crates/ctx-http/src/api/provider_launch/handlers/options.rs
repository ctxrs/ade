use super::*;

mod branches;
mod dispatch;
mod errors;
mod load;
mod probes;

use branches::ProviderOptionsProbeContext;
use dispatch::dispatch_provider_options_probe;
use errors::{
    auth_config_error_provider_options, managed_config_error_provider_options,
    source_config_error_provider_options, unusable_provider_options, ProviderOptionsErrorContext,
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
            ProviderOptionsLoadOutcome::Ready(inputs) => *inputs,
        };
    let ProviderOptionsInputs {
        workspace_id: ws_id,
        install_target,
        launch_config,
        cache,
        workspace,
        preferred_model_id,
        selected_endpoint,
    } = inputs;

    if let Some(config_error) = launch_config.managed_config_error.as_ref() {
        let out = managed_config_error_provider_options(
            ProviderOptionsErrorContext {
                state: &state,
                provider_id: &provider_id,
                workspace_id: ws_id,
                cache: &cache,
                preferred_model_id: preferred_model_id.clone(),
                verify_ttl: VERIFY_TTL,
            },
            config_error,
            launch_config.source_config(),
        )
        .await;
        return Ok(Json(out));
    }

    let provider_status = launch_config
        .provider_status(&state, &provider_id, install_target)
        .await;

    if let Some(config_error) = launch_config.source_config_error.as_ref() {
        let out = source_config_error_provider_options(
            ProviderOptionsErrorContext {
                state: &state,
                provider_id: &provider_id,
                workspace_id: ws_id,
                cache: &cache,
                preferred_model_id: preferred_model_id.clone(),
                verify_ttl: VERIFY_TTL,
            },
            &provider_status,
            config_error,
            launch_config.source_config(),
        )
        .await;
        return Ok(Json(out));
    }

    let source_config = launch_config.source_config();
    let has_active_auth = match probe::provider_has_active_auth_for_workspace_runtime(
        state.as_ref(),
        &workspace,
        &provider_id,
        source_config,
    )
    .await
    {
        Ok(value) => value,
        Err(config_error) => {
            let config_error = logs::redact_sensitive(&config_error);
            let out = auth_config_error_provider_options(
                ProviderOptionsErrorContext {
                    state: &state,
                    provider_id: &provider_id,
                    workspace_id: ws_id,
                    cache: &cache,
                    preferred_model_id: preferred_model_id.clone(),
                    verify_ttl: VERIFY_TTL,
                },
                &provider_status,
                &config_error,
            )
            .await;
            return Ok(Json(out));
        }
    };
    let auth_mode = provider_auth_mode(has_active_auth, source_config);

    if !provider_status_is_usable(&provider_status) {
        let out = unusable_provider_options(
            ProviderOptionsErrorContext {
                state: &state,
                provider_id: &provider_id,
                workspace_id: ws_id,
                cache: &cache,
                preferred_model_id: preferred_model_id.clone(),
                verify_ttl: VERIFY_TTL,
            },
            &provider_status,
            has_active_auth,
            auth_mode,
            source_config,
            selected_endpoint.as_ref(),
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
        source_config,
        selected_endpoint: selected_endpoint.as_ref(),
        cache: &cache,
        preferred_model_id,
        verify_ttl: VERIFY_TTL,
    };

    dispatch_provider_options_probe(use_crp_probe, selected_endpoint.as_ref(), probe_context).await
}
