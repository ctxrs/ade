use ctx_workspace_config as workspace_config;

use super::*;

pub(super) async fn load_bootstrap_workspace(
    state: &Arc<AppState>,
    ws_id: WorkspaceId,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "failed to load workspace",
                })),
            )
        })?;
    if workspace.is_some() {
        return Ok(());
    }

    Err((
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({
            "error": "workspace not found",
        })),
    ))
}

pub(super) async fn load_preferred_model_by_provider(
    state: &Arc<AppState>,
    ws_id: WorkspaceId,
) -> Result<HashMap<String, String>, (StatusCode, Json<serde_json::Value>)> {
    let store = state.store_for_workspace(ws_id).await.map_err(|error| {
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

    workspace_config::load_preferred_new_session_models(&store)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!(
                        "failed to load workspace provider model preferences: {}",
                        logs::redact_sensitive(&error.to_string())
                    ),
                })),
            )
        })
}
