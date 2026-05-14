use super::*;

pub(super) async fn load_bootstrap_workspace(
    providers: &ProvidersHandle,
    ws_id: WorkspaceId,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let exists = providers.workspace_exists(ws_id).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "failed to load workspace",
            })),
        )
    })?;
    if exists {
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
    providers: &ProvidersHandle,
    ws_id: WorkspaceId,
) -> Result<HashMap<String, String>, (StatusCode, Json<serde_json::Value>)> {
    providers
        .load_preferred_new_session_models(ws_id)
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
