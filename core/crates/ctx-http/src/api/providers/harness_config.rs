use super::*;

mod endpoints;

pub(crate) use endpoints::{
    delete_provider_harness_endpoint, refresh_provider_harness_endpoint_models,
    set_provider_harness_endpoint_manual_models, upsert_provider_harness_endpoint,
};

fn provider_harness_bad_request_error(err: anyhow::Error) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": logs::redact_sensitive(&err.to_string()),
        })),
    )
}

pub(crate) async fn get_provider_harness_config(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    let config = providers
        .get_provider_harness_config(&id)
        .await
        .map_err(provider_harness_bad_request_error)?;
    Ok(Json(config))
}

pub(crate) async fn select_provider_harness_source(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
    Json(req): Json<SelectHarnessSourceReq>,
) -> Result<Json<harness_sources::HarnessProviderSourceConfig>, (StatusCode, Json<serde_json::Value>)>
{
    let config = providers
        .select_provider_harness_source(&id, req.source_kind, req.endpoint_id)
        .await
        .map_err(provider_harness_bad_request_error)?;
    Ok(Json(config))
}
