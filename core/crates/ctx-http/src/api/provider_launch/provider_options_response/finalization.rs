use super::*;
use ctx_harness_sources::HarnessEndpointRecord;
use ctx_provider_runtime::provider_launch::models::{
    endpoint_models_payload, subscription_models_payload_from_status,
    supplement_models_payload_with_endpoint_metadata,
};
use ctx_providers::adapters::ProviderStatus;

pub(in crate::api::provider_launch) struct ProviderOptionsResponseContext<'a> {
    pub(in crate::api::provider_launch) state: &'a Arc<AppState>,
    pub(in crate::api::provider_launch) provider_id: &'a str,
    pub(in crate::api::provider_launch) provider_status: Option<&'a ProviderStatus>,
    pub(in crate::api::provider_launch) selected_endpoint: Option<&'a HarnessEndpointRecord>,
    pub(in crate::api::provider_launch) cache: &'a ProviderOptionsCacheSnapshot,
    pub(in crate::api::provider_launch) preferred_model_id: Option<String>,
}

async fn attach_static_provider_models_and_modes(
    state: &Arc<AppState>,
    value: &mut serde_json::Value,
    provider_id: &str,
    provider_status: &ProviderStatus,
    selected_endpoint: Option<&HarnessEndpointRecord>,
    cached_models: Option<serde_json::Value>,
    cached_modes: Option<serde_json::Value>,
) {
    if let Some(endpoint) = selected_endpoint {
        let now = chrono::Utc::now();
        if value.get("models").is_none() || value.get("models").is_some_and(|next| next.is_null()) {
            value["models"] = endpoint_models_payload(provider_id, endpoint, now);
        } else {
            supplement_models_payload_with_endpoint_metadata(
                &mut value["models"],
                provider_id,
                endpoint,
                now,
            );
        }
        if harness_sources::endpoint_model_catalog_is_stale(endpoint, now) {
            let state = Arc::clone(state);
            let provider_id_for_refresh = provider_id.to_string();
            let endpoint_id_for_refresh = endpoint.id.clone();
            tokio::spawn(async move {
                let _ = crate::daemon::providers::refresh_provider_endpoint_model_catalog(
                    &state,
                    &provider_id_for_refresh,
                    &endpoint_id_for_refresh,
                )
                .await;
            });
        }
    } else if value.get("models").is_none()
        || value.get("models").is_some_and(|next| next.is_null())
    {
        if let Some(models) = subscription_models_payload_from_status(provider_status) {
            value["models"] = models;
        }
    }

    if value.get("models").is_none() || value.get("models").is_some_and(|next| next.is_null()) {
        if let Some(models) = cached_models {
            value["models"] = models;
        }
    }
    if value.get("modes").is_none() || value.get("modes").is_some_and(|next| next.is_null()) {
        if let Some(modes) = cached_modes {
            value["modes"] = modes;
        }
    }
}

pub(in crate::api::provider_launch) async fn finalize_provider_options_response(
    context: ProviderOptionsResponseContext<'_>,
    mut raw_response: serde_json::Value,
    write_options_cache: bool,
    verify_ttl: Duration,
) -> serde_json::Value {
    if let Some(provider_status) = context.provider_status {
        attach_static_provider_models_and_modes(
            context.state,
            &mut raw_response,
            context.provider_id,
            provider_status,
            context.selected_endpoint,
            context.cache.cached_models(),
            context.cache.cached_modes(),
        )
        .await;
    }
    inject_preferred_model_id(&mut raw_response, context.preferred_model_id);

    let response = redact_json_value(raw_response);
    if write_options_cache {
        context
            .cache
            .store_response(context.state, response.clone())
            .await;
    }

    let mut out = response;
    context.cache.attach_verify_cache(&mut out, verify_ttl);
    out
}
