use super::*;

pub(in crate::api) async fn verify_provider_for_workspace(
    State(providers): State<ProvidersHandle>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<ProviderAuthCheckRouteResponse>, (StatusCode, Json<serde_json::Value>)> {
    providers
        .verify_provider_for_workspace_for_route(VerifyProviderForWorkspaceRouteRequest {
            workspace_id: ws_id,
            provider_id,
        })
        .await
        .map(Json)
        .map_err(provider_auth_check_route_error)
}
