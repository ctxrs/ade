use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use ctx_harness_sources as harness_sources;
use ctx_observability::logs;

use super::super::types::{SetEndpointManualModelsReq, UpsertHarnessEndpointReq};
use super::provider_harness_bad_request_error;
use crate::daemon::ProvidersHandle;

pub(crate) async fn upsert_provider_harness_endpoint(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
    Json(req): Json<UpsertHarnessEndpointReq>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    let config = providers
        .upsert_provider_harness_endpoint(
            &id,
            harness_sources::HarnessEndpointUpsert {
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
            req.manual_model_ids,
        )
        .await
        .map_err(provider_harness_bad_request_error)?;
    Ok(Json(config))
}

pub(crate) async fn refresh_provider_harness_endpoint_models(
    State(providers): State<ProvidersHandle>,
    Path((id, endpoint_id)): Path<(String, String)>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    let config = providers
        .refresh_provider_harness_endpoint_models(&id, &endpoint_id)
        .await
        .map_err(provider_harness_bad_request_error)?;
    Ok(Json(config))
}

pub(crate) async fn set_provider_harness_endpoint_manual_models(
    State(providers): State<ProvidersHandle>,
    Path((id, endpoint_id)): Path<(String, String)>,
    Json(req): Json<SetEndpointManualModelsReq>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    let config = providers
        .set_provider_harness_endpoint_manual_models(&id, &endpoint_id, req.model_ids)
        .await
        .map_err(provider_harness_bad_request_error)?;
    Ok(Json(config))
}

pub(crate) async fn delete_provider_harness_endpoint(
    State(providers): State<ProvidersHandle>,
    Path((id, endpoint_id)): Path<(String, String)>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    let config = providers
        .delete_provider_harness_endpoint(&id, &endpoint_id)
        .await
        .map_err(provider_harness_delete_error)?;
    Ok(Json(config))
}

fn provider_harness_delete_error(err: anyhow::Error) -> (StatusCode, Json<serde_json::Value>) {
    let error = logs::redact_sensitive(&err.to_string());
    let status = if error.contains("unknown endpoint") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::BAD_REQUEST
    };
    (status, Json(serde_json::json!({ "error": error })))
}
