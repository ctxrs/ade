use super::*;

pub(in crate::api) async fn get_provider_options(
    State(providers): State<ProvidersHandle>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    providers
        .get_provider_options_for_route(ProviderOptionsRouteRequest {
            workspace_id: ws_id,
            provider_id,
        })
        .await
        .map(Json)
        .map_err(provider_options_route_error)
}

fn provider_options_route_error(
    error: ProviderOptionsRouteError,
) -> (StatusCode, Json<serde_json::Value>) {
    let status = match error.status() {
        ProviderOptionsRouteErrorStatus::BadRequest => StatusCode::BAD_REQUEST,
        ProviderOptionsRouteErrorStatus::NotFound => StatusCode::NOT_FOUND,
        ProviderOptionsRouteErrorStatus::InternalServerError => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(error.body().clone()))
}
