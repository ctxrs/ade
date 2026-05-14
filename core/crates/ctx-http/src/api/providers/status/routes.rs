use super::*;
use ctx_daemon::daemon::providers::{parse_provider_install_target, ProviderStatusResponseError};

pub(crate) async fn list_providers(
    State(providers): State<ProvidersHandle>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<Vec<ProviderStatus>>, StatusCode> {
    let target = parse_provider_install_target(query.target.as_deref())
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(
        providers.providers_statuses_response(target, false).await,
    ))
}

pub(crate) async fn get_provider(
    State(providers): State<ProvidersHandle>,
    Path(id): Path<String>,
    Query(query): Query<InstallTargetQuery>,
) -> Result<Json<ProviderStatus>, (StatusCode, Json<serde_json::Value>)> {
    let target = parse_provider_install_target(query.target.as_deref()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )
    })?;

    let status = providers
        .provider_status_response(&id, target)
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
