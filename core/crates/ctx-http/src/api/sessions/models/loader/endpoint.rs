use super::*;

pub(super) enum EndpointModelCatalog {
    Loaded(Option<Box<ModelCatalog>>),
    NotEndpointSource,
}

pub(super) async fn load_endpoint_model_catalog(
    state: &Arc<AppState>,
    workspace: &Workspace,
    provider_id: &str,
    cache_key: String,
) -> Result<EndpointModelCatalog, String> {
    let (source_config, source_config_error) =
        crate::api::provider_launch::load_provider_source_config_with_error(
            &state.core.data_root,
            provider_id,
        )
        .await;
    if let Some(config_error) = source_config_error {
        return Err(config_error);
    }
    let Some(config) = source_config.as_ref() else {
        return Ok(EndpointModelCatalog::NotEndpointSource);
    };
    if config.selected_source_kind != harness_sources::HarnessSourceKind::Endpoint {
        return Ok(EndpointModelCatalog::NotEndpointSource);
    }

    let selected_endpoint_id = config.selected_endpoint_id.as_deref().ok_or_else(|| {
        format!("selected source is endpoint for '{provider_id}' but no endpoint is selected")
    })?;
    let endpoint = config
        .endpoints
        .iter()
        .find(|candidate| candidate.id == selected_endpoint_id)
        .ok_or_else(|| {
            format!("selected endpoint '{selected_endpoint_id}' for '{provider_id}' was not found")
        })?;

    let now = chrono::Utc::now();
    if harness_sources::endpoint_model_catalog_is_stale(endpoint, now) {
        let data_root = state.core.data_root.clone();
        let provider_id_for_refresh = provider_id.to_string();
        let endpoint_id_for_refresh = endpoint.id.clone();
        tokio::spawn(async move {
            let _ = harness_sources::refresh_provider_endpoint_model_catalog(
                &data_root,
                &provider_id_for_refresh,
                &endpoint_id_for_refresh,
            )
            .await;
        });
    }

    let models_value = serde_json::json!({
        "models": endpoint.model_catalog_models,
        "current_model_id": endpoint.model_override,
    });
    let Some(models) = build_model_catalog(&models_value) else {
        return Ok(EndpointModelCatalog::Loaded(None));
    };

    let mut value = serde_json::json!({
        "provider_id": provider_id,
        "workspace_id": workspace.id.0,
        "installed": true,
        "probe_ok": true,
        "supports_load": false,
        "auth_required": false,
        "models": models_value,
        "probed_at": now.to_rfc3339(),
    });
    value["source"] = serde_json::to_value(config).unwrap_or(serde_json::Value::Null);
    value = redact_json_value(value);
    state
        .providers
        .with_provider_options_cache(|cache| {
            cache.insert(
                cache_key,
                crate::daemon::CachedProviderOptions {
                    cached_at: std::time::Instant::now(),
                    value,
                },
            );
        })
        .await;
    Ok(EndpointModelCatalog::Loaded(Some(Box::new(models))))
}
