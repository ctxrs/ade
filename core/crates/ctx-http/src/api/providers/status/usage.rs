use super::*;
use ctx_provider_runtime::provider_usage;

fn provider_usage_internal_error(
    error: impl std::fmt::Display,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": error.to_string()
        })),
    )
}

pub(crate) async fn get_provider_usage(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
    Query(query): Query<ProviderUsageQuery>,
) -> Result<Json<provider_usage::ProviderUsageSnapshot>, (StatusCode, Json<serde_json::Value>)> {
    let refresh = query.refresh.unwrap_or(false);
    let snapshot = providers
        .load_provider_usage(&id, refresh)
        .await
        .map_err(provider_usage_internal_error)?;
    Ok(Json(snapshot))
}
