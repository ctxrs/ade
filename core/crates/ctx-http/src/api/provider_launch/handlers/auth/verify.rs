use super::*;

pub(in crate::api) async fn verify_provider_for_workspace(
    State(providers): State<ProvidersHandle>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<ProviderAuthCheckResp>, (StatusCode, Json<serde_json::Value>)> {
    let ws_id = parse_workspace_id(&ws_id)?;
    providers
        .verify_provider_for_workspace(ws_id, &provider_id)
        .await
        .map(ProviderAuthCheckResp::from)
        .map(Json)
        .map_err(provider_auth_check_error_json)
}
