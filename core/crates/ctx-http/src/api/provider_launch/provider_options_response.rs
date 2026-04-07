use super::*;
use ctx_core::ids::WorkspaceId;
use ctx_providers::adapters::ProviderStatus;

pub(super) fn invalid_provider_id_error(
    provider_id: &str,
    canonical_id: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": format!(
                "provider '{}' is not supported; use '{}'",
                provider_id, canonical_id
            ),
            "code": "invalid_provider_id",
            "provider_id": provider_id,
            "canonical_id": canonical_id,
        })),
    )
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
    crate::workspace_config::load_preferred_new_session_model_id(&store, provider_id)
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
    selected_endpoint: Option<&crate::harness_sources::HarnessEndpointRecord>,
    cached_models: Option<serde_json::Value>,
    cached_modes: Option<serde_json::Value>,
) {
    if let Some(endpoint) = selected_endpoint {
        let now = chrono::Utc::now();
        value["models"] = endpoint_models_payload(provider_id, endpoint, now);
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
