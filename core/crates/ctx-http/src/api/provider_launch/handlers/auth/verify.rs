use super::*;

pub(in crate::api) async fn verify_provider_for_workspace(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<ProviderAuthCheckResp>, (StatusCode, Json<serde_json::Value>)> {
    let ws_id = parse_workspace_id(&ws_id)?;
    crate::daemon::providers::verify_provider_for_workspace(&state, ws_id, &provider_id)
        .await
        .map(ProviderAuthCheckResp::from)
        .map(Json)
        .map_err(provider_auth_check_error_json)
}
