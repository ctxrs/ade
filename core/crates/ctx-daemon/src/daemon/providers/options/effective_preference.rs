use std::sync::Arc;

use ctx_core::models::Workspace;
use ctx_harness_sources::{HarnessEndpointRecord, HarnessProviderSourceConfig};
use ctx_provider_runtime::model_preferences::{
    inject_preferred_model_id, preferred_model_id_from_available_models,
};
use ctx_provider_runtime::provider_auth::provider_auth_mode;
use ctx_provider_runtime::provider_launch::models::{
    endpoint_models_payload, subscription_models_payload_from_status,
};
use ctx_provider_runtime::provider_launch::options::{
    provider_options_probe_plan, provider_supports_runtime_model_catalog,
    runtime_probe_models_payload, ProviderOptionsProbePlan,
};
use ctx_provider_runtime::provider_usability::provider_status_is_usable;
use serde_json::Value;

use crate::daemon::providers;
use crate::daemon::DaemonState;

const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(30);
const VERIFY_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);

#[derive(Debug)]
pub enum EffectivePreferredModelError {
    ExecutionSettings(anyhow::Error),
}

pub async fn effective_preferred_model_id_for_workspace(
    state: &Arc<DaemonState>,
    workspace: &Workspace,
    provider_id: &str,
    preferred_model_id: Option<String>,
) -> Result<Option<String>, EffectivePreferredModelError> {
    let Some(preferred_model_id) = preferred_model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return Ok(None);
    };

    let models =
        effective_model_payload_for_workspace(state, workspace, provider_id, &preferred_model_id)
            .await?;
    Ok(preferred_model_id_from_available_models(
        Some(preferred_model_id),
        models.as_ref(),
    ))
}

async fn effective_model_payload_for_workspace(
    state: &Arc<DaemonState>,
    workspace: &Workspace,
    provider_id: &str,
    preferred_model_id: &str,
) -> Result<Option<Value>, EffectivePreferredModelError> {
    let launch_config = providers::load_provider_launch_config_snapshot(state, provider_id).await;
    if launch_config.managed_config_error.is_some() {
        return Ok(None);
    }

    let install_target = providers::install_target_for_workspace(state, workspace.id)
        .await
        .map_err(EffectivePreferredModelError::ExecutionSettings)?;
    let provider_status = launch_config
        .provider_status(state, provider_id, install_target)
        .await;
    let selected_endpoint = launch_config.selected_endpoint_record();
    let source_config = launch_config.source_config();
    let skip_cached_config_surfaces =
        launch_config.managed_config_error.is_some() || launch_config.source_config_error.is_some();
    let cache = providers::ProviderOptionsCacheSnapshot::load(
        state,
        workspace.id,
        install_target,
        provider_id,
        skip_cached_config_surfaces,
    )
    .await;

    if let Some(cached) = cache.fresh_authoritative_response(CACHE_TTL, VERIFY_TTL) {
        if preferred_model_id_from_available_models(
            Some(preferred_model_id.to_string()),
            cached.get("models"),
        )
        .is_some()
        {
            return Ok(cached.get("models").cloned());
        }
    }

    let active_auth = if launch_config.source_config_error.is_none() {
        providers::provider_has_active_auth_for_workspace_runtime(
            state,
            workspace,
            provider_id,
            source_config,
        )
        .await
        .ok()
    } else {
        None
    };

    if let Some(has_active_auth) = active_auth {
        if let Some(endpoint) = selected_endpoint.as_ref() {
            return Ok(Some(endpoint_models_payload_for_provider(
                provider_id,
                endpoint,
            )));
        }

        if provider_status_is_usable(&provider_status)
            && matches!(
                provider_options_probe_plan(
                    provider_supports_runtime_model_catalog(provider_id),
                    None,
                ),
                ProviderOptionsProbePlan::RuntimeModels
            )
        {
            let probe =
                providers::probe_runtime_models_for_provider_options(state, workspace, provider_id)
                    .await
                    .map_err(EffectivePreferredModelError::ExecutionSettings)?;
            if let Ok(probe) = probe {
                let fallback_current_model_id =
                    subscription_models_payload_from_status(&provider_status).and_then(|models| {
                        models
                            .get("current_model_id")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(str::to_string)
                    });
                if let Some(models) = runtime_probe_models_payload(
                    provider_id,
                    &probe,
                    fallback_current_model_id.as_deref(),
                ) {
                    let response =
                        cached_runtime_models_response(CachedRuntimeModelsResponseArgs {
                            provider_id,
                            workspace_id: workspace.id,
                            installed: provider_status.installed,
                            models: &models,
                            has_active_auth,
                            auth_mode: provider_auth_mode(has_active_auth, source_config),
                            source_config,
                            preferred_model_id,
                        });
                    cache.store_response(state, response).await;
                    return Ok(Some(models));
                }
            }
        }
    }

    Ok(subscription_models_payload_from_status(&provider_status).or_else(|| cache.cached_models()))
}

fn endpoint_models_payload_for_provider(
    provider_id: &str,
    endpoint: &HarnessEndpointRecord,
) -> Value {
    endpoint_models_payload(provider_id, endpoint, chrono::Utc::now())
}

struct CachedRuntimeModelsResponseArgs<'a> {
    provider_id: &'a str,
    workspace_id: ctx_core::ids::WorkspaceId,
    installed: bool,
    models: &'a Value,
    has_active_auth: bool,
    auth_mode: &'a str,
    source_config: Option<&'a HarnessProviderSourceConfig>,
    preferred_model_id: &'a str,
}

fn cached_runtime_models_response(args: CachedRuntimeModelsResponseArgs<'_>) -> Value {
    let CachedRuntimeModelsResponseArgs {
        provider_id,
        workspace_id,
        installed,
        models,
        has_active_auth,
        auth_mode,
        source_config,
        preferred_model_id,
    } = args;
    let mut response = serde_json::json!({
        "provider_id": provider_id,
        "workspace_id": workspace_id.0,
        "installed": installed,
        "probe_ok": true,
        "supports_load": false,
        "auth_required": false,
        "has_active_auth": has_active_auth,
        "auth_mode": auth_mode,
        "probed_at": chrono::Utc::now().to_rfc3339(),
        "models": models,
    });
    if let Some(source_config) = source_config {
        response["source"] = serde_json::to_value(source_config).unwrap_or(Value::Null);
    }
    inject_preferred_model_id(&mut response, Some(preferred_model_id.to_string()));
    response
}
