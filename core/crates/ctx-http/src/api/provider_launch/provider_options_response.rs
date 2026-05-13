use super::*;
use ctx_core::ids::WorkspaceId;

mod finalization;
mod responses;

pub(super) use finalization::{finalize_provider_options_response, ProviderOptionsResponseContext};
pub(super) use responses::{
    config_error_provider_options_response, env_probe_provider_options_response,
    runtime_models_provider_options_response, selected_endpoint_runtime_launch_options_response,
    unusable_provider_options_response, ProviderOptionsProbeResult, ProviderOptionsResponseBase,
};

pub(super) fn parse_workspace_id(
    ws_id: &str,
) -> Result<WorkspaceId, (StatusCode, Json<serde_json::Value>)> {
    Ok(WorkspaceId(uuid::Uuid::parse_str(ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid workspace id",
            })),
        )
    })?))
}

pub(super) async fn load_workspace_preferred_model_id(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    provider_id: &str,
) -> Result<Option<String>, (StatusCode, Json<serde_json::Value>)> {
    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!(
                        "failed to load workspace store: {}",
                        logs::redact_sensitive(&error.to_string())
                    ),
                })),
            )
        })?;
    ctx_workspace_config::load_preferred_new_session_model_id(&store, provider_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!(
                        "failed to load workspace provider model preference: {}",
                        logs::redact_sensitive(&error.to_string())
                    ),
                })),
            )
        })
}
