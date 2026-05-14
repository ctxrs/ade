use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::WorkspaceId;
use ctx_observability::logs;
use ctx_workspace_config as workspace_config;

use crate::api::errors::ApiErrorResp;

pub(super) type PromptConfigResult<T> = Result<T, (StatusCode, Json<ApiErrorResp>)>;

pub(super) fn parse_workspace_id(id: &str) -> PromptConfigResult<WorkspaceId> {
    Ok(WorkspaceId(uuid::Uuid::parse_str(id).map_err(|_| {
        api_error(StatusCode::BAD_REQUEST, "invalid workspace id".to_string())
    })?))
}

pub(super) fn workspace_store_error(
    error: ctx_daemon::daemon::WorkspaceStoreAccessError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        ctx_daemon::daemon::WorkspaceStoreAccessError::NotFound => {
            api_error(StatusCode::NOT_FOUND, "workspace not found".to_string())
        }
        ctx_daemon::daemon::WorkspaceStoreAccessError::Unavailable(error) => {
            api_error(StatusCode::INTERNAL_SERVER_ERROR, redacted(error))
        }
    }
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

fn redacted(err: impl std::fmt::Display) -> String {
    logs::redact_sensitive(&err.to_string())
}

fn api_error(status: StatusCode, error: String) -> (StatusCode, Json<ApiErrorResp>) {
    (status, Json(ApiErrorResp { error }))
}
