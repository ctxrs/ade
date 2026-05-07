use super::*;
use ctx_core::ids::WorkspaceId;
use ctx_harness_sources::HarnessEndpointRecord;
use ctx_provider_runtime::provider_launch::models::{
    endpoint_models_payload, subscription_models_payload_from_status,
    supplement_models_payload_with_endpoint_metadata,
};
use ctx_providers::adapters::ProviderStatus;

pub(super) struct CachedProviderOptionsSnapshot {
    pub(super) cached_at: std::time::Instant,
    pub(super) value: serde_json::Value,
}

pub(super) struct ProviderOptionsCacheSnapshot {
    cache_key: String,
    verify_entry: Option<CachedProviderOptionsSnapshot>,
    authoritative_entry: Option<CachedProviderOptionsSnapshot>,
}

pub(super) struct ProviderOptionsResponseContext<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) provider_id: &'a str,
    pub(super) provider_status: Option<&'a ProviderStatus>,
    pub(super) selected_endpoint: Option<&'a HarnessEndpointRecord>,
    pub(super) cache: &'a ProviderOptionsCacheSnapshot,
    pub(super) preferred_model_id: Option<String>,
}

impl ProviderOptionsCacheSnapshot {
    pub(super) async fn load(
        state: &Arc<AppState>,
        workspace_id: WorkspaceId,
        target: InstallTarget,
        provider_id: &str,
        skip_cached_config_surfaces: bool,
    ) -> Self {
        let cache_key = workspace_provider_cache_key(workspace_id, target, provider_id);
        let verify_entry = if skip_cached_config_surfaces {
            None
        } else {
            state
                .providers
                .verify_cache
                .lock()
                .await
                .get(&cache_key)
                .map(|c| CachedProviderOptionsSnapshot {
                    cached_at: c.cached_at,
                    value: c.value.clone(),
                })
        };
        let cached_entry = if skip_cached_config_surfaces {
            None
        } else {
            state
                .providers
                .options_cache
                .lock()
                .await
                .get(&cache_key)
                .map(|c| CachedProviderOptionsSnapshot {
                    cached_at: c.cached_at,
                    value: c.value.clone(),
                })
        };
        let authoritative_entry = cached_entry
            .filter(|entry| entry.value.get("config_error").is_none())
            .filter(|entry| {
                provider_options_cache_entry_is_authoritative(provider_id, &entry.value)
            });

        Self {
            cache_key,
            verify_entry,
            authoritative_entry,
        }
    }

    pub(super) fn fresh_authoritative_response(
        &self,
        cache_ttl: Duration,
        verify_ttl: Duration,
    ) -> Option<serde_json::Value> {
        let entry = self.authoritative_entry.as_ref()?;
        if entry.cached_at.elapsed() >= cache_ttl {
            return None;
        }
        let mut out = entry.value.clone();
        self.attach_verify_cache(&mut out, verify_ttl);
        Some(out)
    }

    fn cached_payload_field(&self, field: &str) -> Option<serde_json::Value> {
        self.authoritative_entry
            .as_ref()
            .and_then(|entry| entry.value.get(field))
            .cloned()
            .filter(|value| !value.is_null())
    }

    pub(super) fn cached_models(&self) -> Option<serde_json::Value> {
        self.cached_payload_field("models")
    }

    pub(super) fn cached_modes(&self) -> Option<serde_json::Value> {
        self.cached_payload_field("modes")
    }

    pub(super) async fn store_response(&self, state: &Arc<AppState>, value: serde_json::Value) {
        state.providers.options_cache.lock().await.insert(
            self.cache_key.clone(),
            crate::daemon::CachedProviderOptions {
                cached_at: std::time::Instant::now(),
                value,
            },
        );
    }

    pub(super) fn attach_verify_cache(&self, value: &mut serde_json::Value, verify_ttl: Duration) {
        attach_verify_cache(value, self.verify_entry.as_ref(), verify_ttl);
    }
}

pub(super) fn parse_workspace_id(
    ws_id: &str,
) -> Result<WorkspaceId, (StatusCode, Json<serde_json::Value>)> {
    Ok(WorkspaceId(uuid::Uuid::parse_str(ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid workspace id",
            })),
        )
    })?))
}

pub(super) async fn load_workspace_preferred_model_id(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
) -> Result<Option<String>, (StatusCode, Json<serde_json::Value>)> {
    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!(
                        "failed to load workspace store: {}",
                        logs::redact_sensitive(&error.to_string())
                    ),
                })),
            )
        })?;
    ctx_workspace_config::load_preferred_new_session_model_id(&store, provider_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!(
                        "failed to load workspace provider model preference: {}",
                        logs::redact_sensitive(&error.to_string())
                    ),
                })),
            )
        })
}

pub(super) async fn attach_static_provider_models_and_modes(
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
                let _ = harness_sources::refresh_provider_endpoint_model_catalog(
                    &state.core.data_root,
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

pub(super) fn attach_source_config(
    value: &mut serde_json::Value,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) {
    if let Some(source) = source_config {
        value["source"] = serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
    }
}

pub(super) fn config_error_provider_options_response(
    provider_id: &str,
    workspace_id: WorkspaceId,
    installed: Option<bool>,
    auth_mode: &str,
    config_error: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> serde_json::Value {
    let mut response = match installed {
        Some(installed) => serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": workspace_id.0,
            "installed": installed,
            "probe_ok": false,
            "supports_load": false,
            "auth_required": false,
            "has_active_auth": false,
            "auth_mode": auth_mode,
            "probed_at": chrono::Utc::now().to_rfc3339(),
            "probe_error": config_error,
            "config_error": config_error,
        }),
        None => serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": workspace_id.0,
            "probe_ok": false,
            "supports_load": false,
            "auth_required": false,
            "has_active_auth": false,
            "auth_mode": auth_mode,
            "probed_at": chrono::Utc::now().to_rfc3339(),
            "probe_error": config_error,
            "config_error": config_error,
        }),
    };
    attach_source_config(&mut response, source_config);
    response
}

pub(super) fn unusable_provider_options_response(
    provider_id: &str,
    workspace_id: WorkspaceId,
    provider_status: &ProviderStatus,
    has_active_auth: bool,
    auth_mode: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> serde_json::Value {
    let mut response = serde_json::json!({
        "provider_id": provider_id,
        "workspace_id": workspace_id.0,
        "installed": provider_status.installed,
        "health": provider_status.health,
        "diagnostics": provider_status.diagnostics,
        "usability": provider_status.usability,
        "probe_ok": false,
        "probe_error": provider_status_unusable_reason(provider_status)
            .unwrap_or_else(|| "provider not ready for use".to_string()),
        "has_active_auth": has_active_auth,
        "auth_mode": auth_mode,
        "probed_at": chrono::Utc::now().to_rfc3339(),
    });
    attach_source_config(&mut response, source_config);
    response
}

pub(super) fn env_probe_provider_options_response(
    provider_id: &str,
    workspace_id: WorkspaceId,
    provider_status: &ProviderStatus,
    probe_ok: bool,
    auth_required: bool,
    probe_error: Option<String>,
    has_active_auth: bool,
    auth_mode: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> serde_json::Value {
    let mut response = serde_json::json!({
        "provider_id": provider_id,
        "workspace_id": workspace_id.0,
        "installed": provider_status.installed,
        "probe_ok": probe_ok,
        "supports_load": false,
        "auth_required": auth_required,
        "has_active_auth": has_active_auth,
        "auth_mode": auth_mode,
        "probed_at": chrono::Utc::now().to_rfc3339(),
    });
    if let Some(probe_error) = probe_error {
        response["probe_error"] = serde_json::json!(probe_error);
    }
    attach_source_config(&mut response, source_config);
    response
}

pub(super) fn selected_endpoint_runtime_launch_options_response(
    provider_id: &str,
    workspace_id: WorkspaceId,
    provider_status: &ProviderStatus,
    endpoint: &HarnessEndpointRecord,
    probe_ok: bool,
    auth_required: bool,
    probe_error: Option<String>,
    has_active_auth: bool,
    auth_mode: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> serde_json::Value {
    let now = chrono::Utc::now();
    let mut response = serde_json::json!({
        "provider_id": provider_id,
        "workspace_id": workspace_id.0,
        "installed": provider_status.installed,
        "probe_ok": probe_ok,
        "supports_load": false,
        "auth_required": auth_required,
        "has_active_auth": has_active_auth,
        "auth_mode": auth_mode,
        "models": endpoint_models_payload(provider_id, endpoint, now),
        "probed_at": now.to_rfc3339(),
    });
    if let Some(probe_error) = probe_error {
        response["probe_error"] = serde_json::json!(probe_error);
    }
    attach_source_config(&mut response, source_config);
    response
}

pub(super) fn runtime_models_provider_options_response(
    provider_id: &str,
    workspace_id: WorkspaceId,
    provider_status: &ProviderStatus,
    probe: anyhow::Result<ctx_providers::crp::CrpModelsProbe>,
    has_active_auth: bool,
    auth_mode: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> serde_json::Value {
    let mut response = match probe {
        Ok(probe) => {
            let fallback_current_model_id =
                subscription_models_payload_from_status(provider_status).and_then(|models| {
                    models
                        .get("current_model_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                });
            let mut value = serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": workspace_id.0,
                "installed": provider_status.installed,
                "probe_ok": true,
                "supports_load": false,
                "auth_required": false,
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "probed_at": chrono::Utc::now().to_rfc3339(),
            });
            if let Some(models) = runtime_probe_models_payload(
                provider_id,
                &probe,
                fallback_current_model_id.as_deref(),
            ) {
                value["models"] = models;
            } else {
                let probed_at = value
                    .get("probed_at")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let catalog_source = probe.catalog_source.as_deref().unwrap_or("missing");
                let current_model_id = probe.current_model_id.as_deref().unwrap_or("missing");
                let model_count = probe.models.len();
                value = serde_json::json!({
                    "provider_id": provider_id,
                    "workspace_id": workspace_id.0,
                    "installed": provider_status.installed,
                    "probe_ok": false,
                    "probe_error": format!(
                        "runtime_model_catalog_missing: provider={provider_id} catalog_source={catalog_source} current_model_id={current_model_id} model_count={model_count}"
                    ),
                    "auth_required": false,
                    "has_active_auth": has_active_auth,
                    "auth_mode": auth_mode,
                    "probed_at": probed_at,
                    "supports_load": false,
                });
            }
            value
        }
        Err(err) => {
            let probe_error = logs::redact_sensitive(&err.to_string());
            let (_, auth_required, _) = classify_probe_error(&probe_error);
            serde_json::json!({
                "provider_id": provider_id,
                "workspace_id": workspace_id.0,
                "installed": provider_status.installed,
                "probe_ok": false,
                "probe_error": probe_error,
                "auth_required": auth_required.unwrap_or(false),
                "has_active_auth": has_active_auth,
                "auth_mode": auth_mode,
                "probed_at": chrono::Utc::now().to_rfc3339(),
            })
        }
    };
    attach_source_config(&mut response, source_config);
    response
}

pub(super) async fn finalize_provider_options_response(
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

pub(super) fn attach_verify_cache(
    value: &mut serde_json::Value,
    verify_entry: Option<&CachedProviderOptionsSnapshot>,
    verify_ttl: Duration,
) {
    if let Some(verify_entry) = verify_entry {
        if verify_entry.cached_at.elapsed() < verify_ttl {
            if let Some(obj) = value.as_object_mut() {
                obj.insert("verify".to_string(), verify_entry.value.clone());
            }
        }
    }
}
