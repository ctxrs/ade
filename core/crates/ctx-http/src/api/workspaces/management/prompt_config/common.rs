use std::sync::Arc;

use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::WorkspaceId;
use ctx_observability::logs;
use ctx_store::Store;
use ctx_workspace_config as workspace_config;

use crate::api::errors::ApiErrorResp;
use crate::daemon::AppState;

pub(super) type PromptConfigResult<T> = Result<T, (StatusCode, Json<ApiErrorResp>)>;

pub(super) async fn store_for_existing_workspace(
    state: &Arc<AppState>,
    id: &str,
) -> PromptConfigResult<Store> {
    let ws_id = WorkspaceId(
        uuid::Uuid::parse_str(id)
            .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid workspace id".to_string()))?,
    );
    let _workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, redacted(e)))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "workspace not found".to_string()))?;

    state
        .store_for_workspace(ws_id)
        .await
        .map_err(|e| api_error(StatusCode::INTERNAL_SERVER_ERROR, redacted(e)))
}

pub(super) fn source_label(source: workspace_config::AgentSystemPromptAppendSource) -> String {
    match source {
        workspace_config::AgentSystemPromptAppendSource::Default => "default".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Config => "config".to_string(),
        workspace_config::AgentSystemPromptAppendSource::Disabled => "disabled".to_string(),
    }
}

pub(super) fn configured_append(value: &Option<String>) -> Option<String> {
    value.as_ref().map(|value| value.trim().to_string())
}

pub(super) fn bad_request(err: impl std::fmt::Display) -> (StatusCode, Json<ApiErrorResp>) {
    api_error(
        StatusCode::BAD_REQUEST,
        logs::redact_sensitive(&err.to_string()),
    )
}

fn redacted(err: impl std::fmt::Display) -> String {
    logs::redact_sensitive(&err.to_string())
}

fn api_error(status: StatusCode, error: String) -> (StatusCode, Json<ApiErrorResp>) {
    (status, Json(ApiErrorResp { error }))
}
