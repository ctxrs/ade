use super::*;

fn provider_harness_delete_error(err: anyhow::Error) -> (StatusCode, Json<serde_json::Value>) {
    let error = logs::redact_sensitive(&err.to_string());
    let status = if error.contains("unknown endpoint") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::BAD_REQUEST
    };
    (status, Json(serde_json::json!({ "error": error })))
}

pub(crate) async fn get_provider_harness_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    let config = harness_sources::get_provider_source_config(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": logs::redact_sensitive(&e.to_string()),
                })),
            )
        })?;
    Ok(Json(config))
}

pub(crate) async fn select_provider_harness_source(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SelectHarnessSourceReq>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    let config = harness_sources::set_provider_source_selection(
        &state.core.data_root,
        &id,
        req.source_kind,
        req.endpoint_id,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": logs::redact_sensitive(&e.to_string()),
            })),
        )
    })?;
    restarts::invalidate_provider_runtime_state(&state, &id).await;
    Ok(Json(config))
}

pub(crate) async fn upsert_provider_harness_endpoint(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpsertHarnessEndpointReq>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    let endpoint = harness_sources::upsert_provider_endpoint(
        &state.core.data_root,
        &id,
        HarnessEndpointUpsert {
            endpoint_id: req.endpoint_id,
            name: req.name,
            base_url: req.base_url,
            api_shape: req.api_shape,
            auth_type: req.auth_type,
            model_override: req.model_override,
            api_key: req.api_key,
            service_account_json: req.service_account_json,
            project_id: req.project_id,
            location: req.location,
        },
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": logs::redact_sensitive(&e.to_string()),
            })),
        )
    })?;
    if let Some(manual_model_ids) = req.manual_model_ids {
        let _ = harness_sources::set_provider_endpoint_manual_models(
            &state.core.data_root,
            &id,
            &endpoint.id,
            manual_model_ids,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": logs::redact_sensitive(&e.to_string()),
                })),
            )
        })?;
    }
    let _ = harness_sources::refresh_provider_endpoint_model_catalog(
        &state.core.data_root,
        &id,
        &endpoint.id,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": logs::redact_sensitive(&e.to_string()),
            })),
        )
    })?;
    restarts::invalidate_provider_runtime_state(&state, &id).await;
    let config = harness_sources::get_provider_source_config(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": logs::redact_sensitive(&e.to_string()),
                })),
            )
        })?;
    Ok(Json(config))
}

pub(crate) async fn refresh_provider_harness_endpoint_models(
    State(state): State<Arc<AppState>>,
    Path((id, endpoint_id)): Path<(String, String)>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    harness_sources::refresh_provider_endpoint_model_catalog(
        &state.core.data_root,
        &id,
        &endpoint_id,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": logs::redact_sensitive(&e.to_string()),
            })),
        )
    })?;
    restarts::invalidate_provider_runtime_state(&state, &id).await;
    let config = harness_sources::get_provider_source_config(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": logs::redact_sensitive(&e.to_string()),
                })),
            )
        })?;
    Ok(Json(config))
}

pub(crate) async fn set_provider_harness_endpoint_manual_models(
    State(state): State<Arc<AppState>>,
    Path((id, endpoint_id)): Path<(String, String)>,
    Json(req): Json<SetEndpointManualModelsReq>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    harness_sources::set_provider_endpoint_manual_models(
        &state.core.data_root,
        &id,
        &endpoint_id,
        req.model_ids,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": logs::redact_sensitive(&e.to_string()),
            })),
        )
    })?;
    restarts::invalidate_provider_runtime_state(&state, &id).await;
    let config = harness_sources::get_provider_source_config(&state.core.data_root, &id)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": logs::redact_sensitive(&e.to_string()),
                })),
            )
        })?;
    Ok(Json(config))
}

pub(crate) async fn delete_provider_harness_endpoint(
    State(state): State<Arc<AppState>>,
    Path((id, endpoint_id)): Path<(String, String)>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    let config =
        harness_sources::delete_provider_endpoint(&state.core.data_root, &id, &endpoint_id)
            .await
            .map_err(provider_harness_delete_error)?;
    restarts::invalidate_provider_runtime_state(&state, &id).await;
    Ok(Json(config))
}
