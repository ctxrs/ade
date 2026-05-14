use super::*;

mod verify;

pub(in crate::api) use verify::verify_provider_for_workspace;

pub(in crate::api) async fn authenticate_provider_for_workspace(
    State(providers): State<ProvidersHandle>,
    Path((ws_id, provider_id)): Path<(String, String)>,
    req: Option<Json<AuthenticateProviderReq>>,
) -> Result<Json<ProviderAuthCheckResp>, (StatusCode, Json<serde_json::Value>)> {
    let ws_id = parse_workspace_id(&ws_id)?;
    let method_id = req.and_then(|value| value.0.method_id);
    providers
        .authenticate_provider_for_workspace(ws_id, &provider_id, method_id)
        .await
        .map(ProviderAuthCheckResp::from)
        .map(Json)
        .map_err(provider_auth_check_error_json)
}

fn provider_auth_check_error_json(
    error: crate::daemon::providers::ProviderAuthCheckError,
) -> (StatusCode, Json<serde_json::Value>) {
    match error {
        crate::daemon::providers::ProviderAuthCheckError::WorkspaceLoad => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "failed to load workspace",
            })),
        ),
        crate::daemon::providers::ProviderAuthCheckError::WorkspaceNotFound => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "workspace not found",
            })),
        ),
        crate::daemon::providers::ProviderAuthCheckError::ExecutionSettings(error) => {
            workspace_execution_settings_error_json(&error)
        }
        crate::daemon::providers::ProviderAuthCheckError::ProviderLaunchConfig(error) => {
            provider_launch_config_error_response(error)
        }
        crate::daemon::providers::ProviderAuthCheckError::Verify(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": error,
            })),
        ),
    }
}
