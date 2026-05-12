use ctx_core::ids::WorkspaceId;
use ctx_core::models::Workspace;
use ctx_provider_matrix::ProviderMatrix;

use super::*;

pub(super) async fn load_verify_workspace(
    state: &Arc<AppState>,
    ws_id: WorkspaceId,
) -> Result<Workspace, (StatusCode, Json<serde_json::Value>)> {
    state
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
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "workspace not found",
            })),
        ))
}

pub(super) async fn ensure_known_provider(
    state: &Arc<AppState>,
    matrix: &ProviderMatrix,
    provider_id: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let known = state.providers.has_provider_status(provider_id).await
        || ctx_provider_matrix::get_entry(matrix, provider_id).is_some();
    if known {
        return Ok(());
    }

    Err((
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": format!("unsupported provider id: {provider_id}"),
        })),
    ))
}
