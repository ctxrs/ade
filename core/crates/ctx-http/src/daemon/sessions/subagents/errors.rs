use crate::daemon::AppState;
use crate::logs;
use ctx_core::ids::SessionId;
use ctx_core::models::Session;

pub(super) type SubagentResult<T> = Result<T, SubagentError>;
pub(super) type ApiResult<T> = SubagentResult<T>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SubagentErrorKind {
    BadRequest,
    NotFound,
    Forbidden,
    InsufficientStorage,
    Internal,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct SubagentError {
    kind: SubagentErrorKind,
    message: String,
}

impl SubagentError {
    pub(crate) fn kind(&self) -> SubagentErrorKind {
        self.kind
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

pub(super) fn subagent_error(kind: SubagentErrorKind, error: impl Into<String>) -> SubagentError {
    SubagentError {
        kind,
        message: error.into(),
    }
}

pub(super) fn api_error(kind: SubagentErrorKind, error: impl Into<String>) -> SubagentError {
    subagent_error(kind, error)
}

pub(super) fn not_found(error: impl Into<String>) -> SubagentError {
    subagent_error(SubagentErrorKind::NotFound, error)
}

pub(super) fn internal_subagent_error(error: impl ToString) -> SubagentError {
    subagent_error(
        SubagentErrorKind::Internal,
        logs::redact_sensitive(&error.to_string()),
    )
}

pub(super) fn internal_api_error(error: impl ToString) -> SubagentError {
    internal_subagent_error(error)
}

pub(super) fn internal_request_or_policy_error(error: anyhow::Error) -> SubagentError {
    let kind = if ctx_settings_service::is_execution_policy_denial(&error) {
        SubagentErrorKind::Forbidden
    } else if error
        .chain()
        .any(|cause| crate::storage_guard::is_storage_exhaustion_error(&cause.to_string()))
    {
        SubagentErrorKind::InsufficientStorage
    } else {
        SubagentErrorKind::Internal
    };
    subagent_error(kind, logs::redact_sensitive(&format!("{error:#}")))
}

pub(super) async fn store_for_session(
    state: &AppState,
    session_id: SessionId,
) -> SubagentResult<ctx_store::Store> {
    state
        .store_for_session(session_id)
        .await
        .map_err(internal_subagent_error)
}

pub(super) async fn load_parent_session(
    state: &AppState,
    parent_id: SessionId,
) -> SubagentResult<(ctx_store::Store, Session)> {
    let store = store_for_session(state, parent_id).await?;
    if store
        .is_archived_subagent_session(parent_id)
        .await
        .map_err(internal_subagent_error)?
    {
        return Err(not_found("parent session not found"));
    }
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(internal_subagent_error)?
        .ok_or_else(|| not_found("parent session not found"))?;
    Ok((store, parent))
}
