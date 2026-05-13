use super::*;
use crate::daemon::providers::{
    provider_status_response, providers_statuses_response, ProviderStatusResponseError,
};

pub(crate) async fn list_providers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<Vec<ProviderStatus>>, StatusCode> {
    let target = installer::parse_install_target(query.target.as_deref())
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(
        providers_statuses_response(&state, target, false).await,
    ))
}

pub(crate) async fn get_provider(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<ProviderStatus>, (StatusCode, Json<serde_json::Value>)> {
    let target = installer::parse_install_target(query.target.as_deref()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    let status = provider_status_response(&state, &id, target)
        .await
        .map_err(provider_status_response_error)?;
    Ok(Json(status))
}

fn provider_status_response_error(
    error: ProviderStatusResponseError,
) -> (StatusCode, Json<serde_json::Value>) {
    match error {
        ProviderStatusResponseError::NotFound { provider_id } => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("provider not found: {provider_id}")
            })),
        ),
    }
}
