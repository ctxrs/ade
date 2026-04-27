use super::*;
use ctx_core::ids::WorkspaceId;
use ctx_harness_sources::HarnessEndpointRecord;
use ctx_providers::adapters::ProviderStatus;

fn supplement_models_payload_with_endpoint_metadata(
    models: &mut serde_json::Value,
    provider_id: &str,
    endpoint: &HarnessEndpointRecord,
    now: chrono::DateTime<chrono::Utc>,
) {
    let endpoint_payload = endpoint_models_payload(provider_id, endpoint, now);
    let Some(models_obj) = models.as_object_mut() else {
        *models = endpoint_payload;
        return;
    };

    let missing_current_model = models_obj
        .get("current_model_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .map(str::is_empty)
        .unwrap_or(true);
    if missing_current_model {
        if let Some(current_model_id) = endpoint_payload
            .get("current_model_id")
            .cloned()
            .filter(|value| !value.is_null())
        {
            models_obj.insert("current_model_id".to_string(), current_model_id);
        }
    }

    let endpoint_meta = endpoint_payload
        .get("meta")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    match models_obj.get_mut("meta") {
        Some(serde_json::Value::Object(meta_obj)) => {
            meta_obj.insert("endpoint".to_string(), endpoint_meta);
        }
        _ => {
            models_obj.insert(
                "meta".to_string(),
                serde_json::json!({
                    "endpoint": endpoint_meta,
                }),
            );
        }
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

pub(super) fn inject_preferred_model_id(
    value: &mut serde_json::Value,
    preferred_model_id: Option<String>,
) {
    let resolved = crate::provider_model_preferences::preferred_model_id_from_available_models(
        preferred_model_id,
        value.get("models"),
    );
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if let Some(preferred_model_id) = resolved {
        obj.insert(
            "preferred_model_id".to_string(),
            serde_json::json!(preferred_model_id),
        );
    } else {
        obj.remove("preferred_model_id");
    }
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

pub(super) fn attach_verify_cache(
    value: &mut serde_json::Value,
    verify_entry: Option<&(std::time::Instant, serde_json::Value)>,
    verify_ttl: Duration,
) {
    if let Some((verify_at, verify)) = verify_entry {
        if verify_at.elapsed() < verify_ttl {
            if let Some(obj) = value.as_object_mut() {
                obj.insert("verify".to_string(), verify.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ctx_harness_sources::{
        EndpointModelCatalogStatus, EndpointModelRecord, HarnessApiShape, HarnessEndpointRecord,
        HarnessEndpointVerificationStatus,
    };

    fn test_endpoint() -> HarnessEndpointRecord {
        HarnessEndpointRecord {
            id: "ep-1".to_string(),
            provider_id: "codex".to_string(),
            name: "OpenRouter".to_string(),
            base_url: Some("https://openrouter.ai/api/v1".to_string()),
            api_shape: HarnessApiShape::OpenaiResponses,
            auth_type: "bearer".to_string(),
            model_override: Some("openai/gpt-5.2".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_verification_status: HarnessEndpointVerificationStatus::Valid,
            last_verification_at: None,
            last_error: None,
            has_api_key: true,
            model_catalog_status: EndpointModelCatalogStatus::Ready,
            model_catalog_fetched_at: Some(Utc::now()),
            model_catalog_error: None,
            model_catalog_models: vec![EndpointModelRecord {
                id: "openai/gpt-5.2".to_string(),
                name: Some("GPT-5.2".to_string()),
            }],
            manual_model_ids: vec!["manual/fallback".to_string()],
            model_catalog_source: Some("mixed".to_string()),
        }
    }

    #[test]
    fn supplement_models_payload_preserves_live_probe_catalog() {
        let endpoint = test_endpoint();
        let now = Utc::now();
        let mut models = serde_json::json!({
            "models": [
                { "id": "openai/gpt-5.4", "name": "GPT-5.4" },
                { "id": "openai/o3", "name": "o3" }
            ],
            "current_model_id": "openai/gpt-5.4",
            "meta": {
                "source_kind": "subscription",
                "catalog_source": "runtime_probe_live",
                "refresh_pending": false,
            },
        });

        supplement_models_payload_with_endpoint_metadata(&mut models, "codex", &endpoint, now);

        let model_ids = models
            .get("models")
            .and_then(serde_json::Value::as_array)
            .expect("models array")
            .iter()
            .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(model_ids, vec!["openai/gpt-5.4", "openai/o3"]);
        assert_eq!(
            models
                .get("current_model_id")
                .and_then(serde_json::Value::as_str),
            Some("openai/gpt-5.4")
        );
        assert_eq!(
            models
                .pointer("/meta/source_kind")
                .and_then(serde_json::Value::as_str),
            Some("subscription")
        );
        assert_eq!(
            models
                .pointer("/meta/endpoint/catalog_status")
                .and_then(serde_json::Value::as_str),
            Some("ready")
        );
        assert_eq!(
            models
                .pointer("/meta/endpoint/catalog_source")
                .and_then(serde_json::Value::as_str),
            Some("mixed")
        );
    }

    #[test]
    fn supplement_models_payload_uses_endpoint_current_model_when_probe_has_none() {
        let endpoint = test_endpoint();
        let now = Utc::now();
        let mut models = serde_json::json!({
            "models": [
                { "id": "openai/gpt-5.4", "name": "GPT-5.4" }
            ],
            "meta": {
                "source_kind": "subscription",
                "catalog_source": "runtime_probe_live",
                "refresh_pending": false,
            },
        });

        supplement_models_payload_with_endpoint_metadata(&mut models, "codex", &endpoint, now);

        assert_eq!(
            models
                .get("current_model_id")
                .and_then(serde_json::Value::as_str),
            Some("openai/gpt-5.2")
        );
    }
}
