use super::*;

use ctx_daemon::daemon::providers::{
    ProvidersBootstrapError, ProvidersBootstrapErrorKind, ProvidersBootstrapResponse,
};

fn parse_workspace_id(ws_id: &str) -> Result<WorkspaceId, (StatusCode, Json<serde_json::Value>)> {
    Ok(WorkspaceId(uuid::Uuid::parse_str(ws_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid workspace id",
            })),
        )
    })?))
}

pub(crate) async fn get_workspace_providers_bootstrap(
    State(providers): State<ProvidersHandle>,
    Path(ws_id): Path<String>,
) -> Result<Json<ProvidersBootstrapResponse>, (StatusCode, Json<serde_json::Value>)> {
    let ws_id = parse_workspace_id(&ws_id)?;
    providers
        .workspace_providers_bootstrap(ws_id)
        .await
        .map(Json)
        .map_err(provider_bootstrap_error_json)
}

fn provider_bootstrap_error_json(
    error: ProvidersBootstrapError,
) -> (StatusCode, Json<serde_json::Value>) {
    let status = match error.kind() {
        ProvidersBootstrapErrorKind::NotFound => StatusCode::NOT_FOUND,
        ProvidersBootstrapErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(serde_json::json!({
            "error": error.message(),
        })),
    )
}
