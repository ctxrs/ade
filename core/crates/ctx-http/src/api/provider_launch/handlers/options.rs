use super::*;

pub(in crate::api) async fn get_provider_options(
    State(state): State<Arc<AppState>>,
    Path((ws_id, provider_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let workspace_id = parse_workspace_id(&ws_id)?;
    crate::daemon::providers::get_provider_options_response(&state, workspace_id, &provider_id)
        .await
        .map(Json)
        .map_err(provider_options_response_error_json)
}

fn provider_options_response_error_json(
    error: crate::daemon::providers::ProviderOptionsResponseError,
) -> (StatusCode, Json<serde_json::Value>) {
    match error {
        crate::daemon::providers::ProviderOptionsResponseError::ExecutionSettings(error) => {
            workspace_execution_settings_error_json(&error)
        }
        crate::daemon::providers::ProviderOptionsResponseError::ProviderLaunchConfig(error) => {
            provider_launch_config_error_response(error)
        }
        crate::daemon::providers::ProviderOptionsResponseError::WorkspaceLoad => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "failed to load workspace",
            })),
        ),
        crate::daemon::providers::ProviderOptionsResponseError::WorkspaceNotFound => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "workspace not found",
            })),
        ),
        crate::daemon::providers::ProviderOptionsResponseError::WorkspaceStoreLoad(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!(
                    "failed to load workspace store: {}",
                    logs::redact_sensitive(&error.to_string())
                ),
            })),
        ),
        crate::daemon::providers::ProviderOptionsResponseError::WorkspacePreferenceLoad(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!(
                    "failed to load workspace provider model preference: {}",
                    logs::redact_sensitive(&error.to_string())
                ),
            })),
        ),
        crate::daemon::providers::ProviderOptionsResponseError::SelectedEndpointMissing => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "selected endpoint missing from provider configuration",
            })),
        ),
    }
}
