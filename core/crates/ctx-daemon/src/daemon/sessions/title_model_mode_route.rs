use ctx_core::ids::SessionId;
use ctx_core::models::Session;
use ctx_observability::logs;
use serde::{Deserialize, Serialize};

use crate::daemon::sessions::route_contract::parse_session_route_id;
use crate::daemon::sessions::{
    GenerateSessionTitleError, SetSessionModeError, SetSessionModelError, SetSessionModelErrorKind,
};
use crate::daemon::{SessionRouteParams, SessionsHandle};

#[derive(Debug, Clone, Deserialize)]
pub struct GenerateSessionTitleRouteRequest {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    force: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct GenerateSessionTitleRouteResponse(Session);

#[derive(Debug, Clone, Deserialize)]
pub struct SetSessionModelRouteRequest {
    model_id: String,
    #[serde(default)]
    reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct SetSessionModelRouteResponse(Session);

#[derive(Debug, Clone, Deserialize)]
pub struct SetSessionModeRouteRequest {
    mode_id: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SessionTitleModelModeRouteErrorKind {
    BadRequest,
    NotFound,
    Forbidden,
    InsufficientStorage,
    ProviderUnavailable,
    LiveSwitchRejected,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SessionTitleModelModeRouteError {
    kind: SessionTitleModelModeRouteErrorKind,
    message: String,
}

impl SessionTitleModelModeRouteError {
    fn new(kind: SessionTitleModelModeRouteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(SessionTitleModelModeRouteErrorKind::BadRequest, message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(SessionTitleModelModeRouteErrorKind::NotFound, message)
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self::new(SessionTitleModelModeRouteErrorKind::Forbidden, message)
    }

    fn insufficient_storage(message: impl Into<String>) -> Self {
        Self::new(
            SessionTitleModelModeRouteErrorKind::InsufficientStorage,
            message,
        )
    }

    fn provider_unavailable(message: impl Into<String>) -> Self {
        Self::new(
            SessionTitleModelModeRouteErrorKind::ProviderUnavailable,
            message,
        )
    }

    fn live_switch_rejected(message: impl Into<String>) -> Self {
        Self::new(
            SessionTitleModelModeRouteErrorKind::LiveSwitchRejected,
            message,
        )
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(SessionTitleModelModeRouteErrorKind::Internal, message)
    }

    pub fn kind(&self) -> SessionTitleModelModeRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl SessionsHandle {
    pub async fn generate_session_title_for_route(
        &self,
        params: SessionRouteParams,
        request: GenerateSessionTitleRouteRequest,
    ) -> Result<GenerateSessionTitleRouteResponse, SessionTitleModelModeRouteError> {
        let session_id = parse_title_model_mode_session_id(params)?;
        self.generate_session_title_for_request(session_id, request.prompt, request.force)
            .await
            .map(GenerateSessionTitleRouteResponse)
            .map_err(generate_title_route_error)
    }

    pub async fn set_session_model_for_route(
        &self,
        params: SessionRouteParams,
        request: SetSessionModelRouteRequest,
    ) -> Result<SetSessionModelRouteResponse, SessionTitleModelModeRouteError> {
        let session_id = parse_title_model_mode_session_id(params)?;
        self.set_session_model_for_request(
            session_id,
            crate::daemon::sessions::SetSessionModelRequest {
                model_id: request.model_id,
                reasoning_effort: request.reasoning_effort,
            },
        )
        .await
        .map(SetSessionModelRouteResponse)
        .map_err(set_model_route_error)
    }

    pub async fn set_session_mode_for_route(
        &self,
        params: SessionRouteParams,
        request: SetSessionModeRouteRequest,
    ) -> Result<(), SessionTitleModelModeRouteError> {
        let session_id = parse_title_model_mode_session_id(params)?;
        self.set_session_mode_for_request(session_id, request.mode_id)
            .await
            .map_err(set_mode_route_error)
    }
}

fn parse_title_model_mode_session_id(
    params: SessionRouteParams,
) -> Result<SessionId, SessionTitleModelModeRouteError> {
    parse_session_route_id(params.session_id())
        .map_err(|_| SessionTitleModelModeRouteError::bad_request("invalid session id"))
}

fn generate_title_route_error(error: GenerateSessionTitleError) -> SessionTitleModelModeRouteError {
    match error {
        GenerateSessionTitleError::NotFound => {
            SessionTitleModelModeRouteError::not_found("session not found")
        }
        GenerateSessionTitleError::PromptRequired => {
            SessionTitleModelModeRouteError::bad_request("prompt required")
        }
        GenerateSessionTitleError::Skipped => {
            SessionTitleModelModeRouteError::bad_request("title generation skipped")
        }
        GenerateSessionTitleError::Internal(error) => {
            SessionTitleModelModeRouteError::internal(logs::redact_sensitive(&error.to_string()))
        }
    }
}

fn set_model_route_error(error: SetSessionModelError) -> SessionTitleModelModeRouteError {
    match error.kind() {
        SetSessionModelErrorKind::BadRequest => {
            SessionTitleModelModeRouteError::bad_request(error.message())
        }
        SetSessionModelErrorKind::NotFound => {
            SessionTitleModelModeRouteError::not_found(error.message())
        }
        SetSessionModelErrorKind::Forbidden => {
            SessionTitleModelModeRouteError::forbidden(error.message())
        }
        SetSessionModelErrorKind::InsufficientStorage => {
            SessionTitleModelModeRouteError::insufficient_storage(error.message())
        }
        SetSessionModelErrorKind::ProviderUnavailable => {
            SessionTitleModelModeRouteError::provider_unavailable(error.message())
        }
        SetSessionModelErrorKind::LiveSwitchRejected => {
            SessionTitleModelModeRouteError::live_switch_rejected(error.message())
        }
        SetSessionModelErrorKind::Internal => {
            SessionTitleModelModeRouteError::internal(error.message())
        }
    }
}

fn set_mode_route_error(error: SetSessionModeError) -> SessionTitleModelModeRouteError {
    match error {
        SetSessionModeError::NotFound => {
            SessionTitleModelModeRouteError::not_found("session not found")
        }
        SetSessionModeError::BadRequest => {
            SessionTitleModelModeRouteError::bad_request("bad request")
        }
        SetSessionModeError::Internal => {
            SessionTitleModelModeRouteError::internal("internal server error")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ctx_core::ids::{TaskId, WorkspaceId, WorktreeId};
    use ctx_core::models::{ExecutionEnvironment, SessionStatus};
    use serde_json::json;

    fn session() -> Session {
        Session {
            id: SessionId::new(),
            task_id: TaskId::new(),
            workspace_id: WorkspaceId::new(),
            worktree_id: WorktreeId::new(),
            execution_environment: ExecutionEnvironment::Host,
            parent_session_id: None,
            relationship: None,
            provider_id: "fake".to_string(),
            model_id: "model".to_string(),
            reasoning_effort: Some("high".to_string()),
            title: "Title".to_string(),
            agent_role: "dev".to_string(),
            status: SessionStatus::Active,
            provider_session_ref: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn title_response_preserves_session_wire_shape() {
        let session = session();
        assert_eq!(
            serde_json::to_value(GenerateSessionTitleRouteResponse(session.clone())).unwrap(),
            serde_json::to_value(session).unwrap()
        );
    }

    #[test]
    fn model_response_preserves_session_wire_shape() {
        let session = session();
        assert_eq!(
            serde_json::to_value(SetSessionModelRouteResponse(session.clone())).unwrap(),
            serde_json::to_value(session).unwrap()
        );
    }

    #[test]
    fn route_requests_preserve_current_serde_shape() {
        let title: GenerateSessionTitleRouteRequest = serde_json::from_value(json!({
            "prompt": "hello",
            "force": false,
            "ignored": true
        }))
        .unwrap();
        assert_eq!(title.prompt.as_deref(), Some("hello"));
        assert_eq!(title.force, Some(false));

        let title_defaults: GenerateSessionTitleRouteRequest =
            serde_json::from_value(json!({ "ignored": true })).unwrap();
        assert_eq!(title_defaults.prompt, None);
        assert_eq!(title_defaults.force, None);

        let model: SetSessionModelRouteRequest = serde_json::from_value(json!({
            "model_id": "codex/gpt-5",
            "reasoning_effort": "high",
            "ignored": true
        }))
        .unwrap();
        assert_eq!(model.model_id, "codex/gpt-5");
        assert_eq!(model.reasoning_effort.as_deref(), Some("high"));

        let mode: SetSessionModeRouteRequest =
            serde_json::from_value(json!({ "mode_id": "planning", "ignored": true })).unwrap();
        assert_eq!(mode.mode_id, "planning");
    }

    #[test]
    fn invalid_session_id_uses_existing_route_message() {
        let error = parse_title_model_mode_session_id(SessionRouteParams::new("not-a-session"))
            .unwrap_err();
        assert_eq!(
            error.kind(),
            SessionTitleModelModeRouteErrorKind::BadRequest
        );
        assert_eq!(error.message(), "invalid session id");
    }

    #[test]
    fn title_errors_preserve_messages_and_redact_internal() {
        assert_eq!(
            generate_title_route_error(GenerateSessionTitleError::NotFound).message(),
            "session not found"
        );
        assert_eq!(
            generate_title_route_error(GenerateSessionTitleError::PromptRequired).message(),
            "prompt required"
        );
        assert_eq!(
            generate_title_route_error(GenerateSessionTitleError::Skipped).message(),
            "title generation skipped"
        );
        let raw_message = "CTX_MCP_TOKEN=secret-token-123";
        let error = generate_title_route_error(GenerateSessionTitleError::Internal(
            anyhow::anyhow!(raw_message),
        ));
        assert_eq!(error.kind(), SessionTitleModelModeRouteErrorKind::Internal);
        assert_eq!(error.message(), logs::redact_sensitive(raw_message));
        assert!(!error.message().contains("secret-token-123"));
    }

    #[test]
    fn model_error_kind_mapping_preserves_status_categories() {
        for (kind, expected_kind) in [
            (
                SetSessionModelErrorKind::BadRequest,
                SessionTitleModelModeRouteErrorKind::BadRequest,
            ),
            (
                SetSessionModelErrorKind::NotFound,
                SessionTitleModelModeRouteErrorKind::NotFound,
            ),
            (
                SetSessionModelErrorKind::Forbidden,
                SessionTitleModelModeRouteErrorKind::Forbidden,
            ),
            (
                SetSessionModelErrorKind::InsufficientStorage,
                SessionTitleModelModeRouteErrorKind::InsufficientStorage,
            ),
            (
                SetSessionModelErrorKind::ProviderUnavailable,
                SessionTitleModelModeRouteErrorKind::ProviderUnavailable,
            ),
            (
                SetSessionModelErrorKind::LiveSwitchRejected,
                SessionTitleModelModeRouteErrorKind::LiveSwitchRejected,
            ),
            (
                SetSessionModelErrorKind::Internal,
                SessionTitleModelModeRouteErrorKind::Internal,
            ),
        ] {
            let error = set_model_route_error(SetSessionModelError::new(kind, "model message"));
            assert_eq!(error.kind(), expected_kind);
            assert_eq!(error.message(), "model message");
        }
    }

    #[test]
    fn mode_error_mapping_preserves_bare_status_categories() {
        assert_eq!(
            set_mode_route_error(SetSessionModeError::NotFound).kind(),
            SessionTitleModelModeRouteErrorKind::NotFound
        );
        assert_eq!(
            set_mode_route_error(SetSessionModeError::BadRequest).kind(),
            SessionTitleModelModeRouteErrorKind::BadRequest
        );
        assert_eq!(
            set_mode_route_error(SetSessionModeError::Internal).kind(),
            SessionTitleModelModeRouteErrorKind::Internal
        );
    }
}
