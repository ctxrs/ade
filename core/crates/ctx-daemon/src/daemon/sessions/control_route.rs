use std::collections::HashMap;
use std::time::Instant;

use ctx_observability::logs;
use ctx_providers::ask_user_question::AskUserQuestionOutcome;
use serde::{Deserialize, Serialize};

use crate::daemon::sessions::ask_user::{SubmitAskUserAnswer, SubmitAskUserAnswerError};
use crate::daemon::sessions::auth::SessionAuthError;
use crate::daemon::sessions::command_dispatch::SessionSchedulerCommandError;
use crate::daemon::sessions::route_contract::parse_session_route_id;
use crate::daemon::workspaces::FileCompletionsErrorKind;
use crate::daemon::{SessionRouteParams, SessionsHandle};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthenticateSessionRouteRequest {
    #[serde(default)]
    method_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubmitAskUserQuestionRouteRequest {
    tool_call_id: String,
    #[serde(default)]
    outcome: Option<String>,
    #[serde(default)]
    answers: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubmitAskUserQuestionRouteResponse {
    ok: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionFileCompletionsRouteQuery {
    query: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct SessionFileCompletionsRouteResponse(Vec<String>);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SessionControlRouteErrorKind {
    BadRequest,
    NotFound,
    Forbidden,
    Conflict,
    InsufficientStorage,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SessionControlRouteError {
    kind: SessionControlRouteErrorKind,
    message: String,
}

impl SessionControlRouteError {
    pub fn new(kind: SessionControlRouteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(SessionControlRouteErrorKind::BadRequest, message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(SessionControlRouteErrorKind::NotFound, message)
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self::new(SessionControlRouteErrorKind::Forbidden, message)
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self::new(SessionControlRouteErrorKind::Conflict, message)
    }

    fn insufficient_storage(message: impl Into<String>) -> Self {
        Self::new(SessionControlRouteErrorKind::InsufficientStorage, message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(SessionControlRouteErrorKind::Internal, message)
    }

    pub fn kind(&self) -> SessionControlRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl SessionsHandle {
    pub async fn cancel_session_for_route(
        &self,
        params: SessionRouteParams,
    ) -> Result<(), SessionControlRouteError> {
        let session_id = parse_control_session_id(params)?;
        self.cancel_session(session_id)
            .await
            .map_err(scheduler_command_error)
    }

    pub async fn interrupt_session_for_route(
        &self,
        params: SessionRouteParams,
        request_started: Instant,
    ) -> Result<(), SessionControlRouteError> {
        let session_id = parse_control_session_id(params)?;
        self.interrupt_session(session_id, request_started)
            .await
            .map_err(scheduler_command_error)
    }

    pub async fn authenticate_session_for_route(
        &self,
        params: SessionRouteParams,
        request: AuthenticateSessionRouteRequest,
    ) -> Result<(), SessionControlRouteError> {
        let session_id = parse_control_session_id(params)?;
        self.authenticate_session_for_request(session_id, request.method_id)
            .await
            .map_err(session_auth_error)
    }

    pub async fn submit_ask_user_question_for_route(
        &self,
        params: SessionRouteParams,
        request: SubmitAskUserQuestionRouteRequest,
    ) -> Result<SubmitAskUserQuestionRouteResponse, SessionControlRouteError> {
        let session_id = parse_control_session_id(params)?;
        let outcome = match request.outcome.as_deref() {
            Some("cancelled") => AskUserQuestionOutcome::Cancelled,
            Some("submitted") | None => AskUserQuestionOutcome::Submitted,
            Some(other) => {
                return Err(SessionControlRouteError::bad_request(format!(
                    "invalid outcome: {other}"
                )));
            }
        };

        self.submit_ask_user_answer(
            session_id,
            SubmitAskUserAnswer {
                tool_call_id: request.tool_call_id,
                outcome,
                answers: request.answers.unwrap_or_default(),
            },
        )
        .await
        .map_err(submit_ask_user_answer_error)?;

        Ok(SubmitAskUserQuestionRouteResponse { ok: true })
    }

    pub async fn complete_files_for_session_for_route(
        &self,
        params: SessionRouteParams,
        query: SessionFileCompletionsRouteQuery,
    ) -> Result<SessionFileCompletionsRouteResponse, SessionControlRouteError> {
        let session_id = parse_control_session_id(params)?;
        self.complete_files_for_session(session_id, query.query, query.limit)
            .await
            .map(SessionFileCompletionsRouteResponse)
            .map_err(file_completions_error)
    }
}

fn parse_control_session_id(
    params: SessionRouteParams,
) -> Result<ctx_core::ids::SessionId, SessionControlRouteError> {
    parse_session_route_id(params.session_id())
        .map_err(|_| SessionControlRouteError::bad_request("invalid session id"))
}

fn scheduler_command_error(error: SessionSchedulerCommandError) -> SessionControlRouteError {
    match error {
        SessionSchedulerCommandError::BadRequest => {
            SessionControlRouteError::bad_request("bad request")
        }
        SessionSchedulerCommandError::NotFound => {
            SessionControlRouteError::not_found("session not found")
        }
        SessionSchedulerCommandError::StoreUnavailable => {
            SessionControlRouteError::internal("session store unavailable")
        }
    }
}

fn session_auth_error(error: SessionAuthError) -> SessionControlRouteError {
    match error {
        SessionAuthError::NotFound(entity) => {
            SessionControlRouteError::not_found(format!("{entity} not found"))
        }
        SessionAuthError::BadRequest(error) => SessionControlRouteError::bad_request(error),
        SessionAuthError::Forbidden(error) => SessionControlRouteError::forbidden(error),
        SessionAuthError::Internal(error) => SessionControlRouteError::internal(error),
        SessionAuthError::AuthenticationFailed { redacted_message } => {
            tracing::warn!("session authentication failed: {redacted_message}");
            SessionControlRouteError::bad_request("authentication failed")
        }
    }
}

fn submit_ask_user_answer_error(error: SubmitAskUserAnswerError) -> SessionControlRouteError {
    match error {
        SubmitAskUserAnswerError::MissingToolCallId => {
            SessionControlRouteError::bad_request("missing tool_call_id")
        }
        SubmitAskUserAnswerError::SessionNotFound => {
            SessionControlRouteError::not_found("session not found")
        }
        SubmitAskUserAnswerError::StoreUnavailable(err) => {
            SessionControlRouteError::internal(logs::redact_sensitive(&err.to_string()))
        }
        SubmitAskUserAnswerError::LoadSession => {
            SessionControlRouteError::internal("failed to load session")
        }
        SubmitAskUserAnswerError::NoPendingQuestion => {
            SessionControlRouteError::conflict("no pending AskUserQuestion for this tool_call_id")
        }
    }
}

fn file_completions_error(
    error: crate::daemon::workspaces::FileCompletionsError,
) -> SessionControlRouteError {
    match error.kind() {
        FileCompletionsErrorKind::NotFound => {
            SessionControlRouteError::not_found(error.message().to_string())
        }
        FileCompletionsErrorKind::Forbidden => {
            SessionControlRouteError::forbidden(error.message().to_string())
        }
        FileCompletionsErrorKind::InsufficientStorage => {
            SessionControlRouteError::insufficient_storage(error.message().to_string())
        }
        FileCompletionsErrorKind::Internal => {
            tracing::warn!(error = error.message(), "file completions request failed");
            SessionControlRouteError::internal(error.message().to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ask_user_request_parses_default_and_cancelled_outcomes() {
        let submitted: SubmitAskUserQuestionRouteRequest =
            serde_json::from_value(json!({"tool_call_id": "tool-1"})).unwrap();
        assert!(matches!(
            submitted.outcome.as_deref().unwrap_or("submitted"),
            "submitted"
        ));

        let cancelled: SubmitAskUserQuestionRouteRequest = serde_json::from_value(json!({
            "tool_call_id": "tool-1",
            "outcome": "cancelled",
            "answers": {"choice": "no"}
        }))
        .unwrap();
        assert_eq!(cancelled.outcome.as_deref(), Some("cancelled"));
        assert_eq!(
            cancelled.answers.unwrap().get("choice").map(String::as_str),
            Some("no")
        );
    }

    #[test]
    fn control_responses_preserve_wire_shapes() {
        assert_eq!(
            serde_json::to_value(SubmitAskUserQuestionRouteResponse { ok: true }).unwrap(),
            json!({"ok": true})
        );

        let completions =
            SessionFileCompletionsRouteResponse(vec!["src/lib.rs".to_string(), "README.md".into()]);
        assert_eq!(
            serde_json::to_value(completions).unwrap(),
            json!(["src/lib.rs", "README.md"])
        );
    }

    #[test]
    fn control_route_errors_classify_existing_status_categories() {
        assert_eq!(
            scheduler_command_error(SessionSchedulerCommandError::BadRequest).kind(),
            SessionControlRouteErrorKind::BadRequest
        );
        assert_eq!(
            scheduler_command_error(SessionSchedulerCommandError::NotFound).kind(),
            SessionControlRouteErrorKind::NotFound
        );
        assert_eq!(
            scheduler_command_error(SessionSchedulerCommandError::StoreUnavailable).kind(),
            SessionControlRouteErrorKind::Internal
        );
    }

    #[test]
    fn auth_errors_preserve_user_facing_messages() {
        let not_found = session_auth_error(SessionAuthError::NotFound("session"));
        assert_eq!(not_found.kind(), SessionControlRouteErrorKind::NotFound);
        assert_eq!(not_found.message(), "session not found");

        let forbidden = session_auth_error(SessionAuthError::Forbidden(
            "host execution is disabled by daemon policy".to_string(),
        ));
        assert_eq!(forbidden.kind(), SessionControlRouteErrorKind::Forbidden);
        assert_eq!(
            forbidden.message(),
            "host execution is disabled by daemon policy"
        );

        let failed = session_auth_error(SessionAuthError::AuthenticationFailed {
            redacted_message: "provider rejected credentials".to_string(),
        });
        assert_eq!(failed.kind(), SessionControlRouteErrorKind::BadRequest);
        assert_eq!(failed.message(), "authentication failed");
    }

    #[test]
    fn ask_user_errors_preserve_user_facing_messages() {
        let missing = submit_ask_user_answer_error(SubmitAskUserAnswerError::MissingToolCallId);
        assert_eq!(missing.kind(), SessionControlRouteErrorKind::BadRequest);
        assert_eq!(missing.message(), "missing tool_call_id");

        let conflict = submit_ask_user_answer_error(SubmitAskUserAnswerError::NoPendingQuestion);
        assert_eq!(conflict.kind(), SessionControlRouteErrorKind::Conflict);
        assert_eq!(
            conflict.message(),
            "no pending AskUserQuestion for this tool_call_id"
        );
    }

    #[test]
    fn session_id_parser_reuses_route_contract_semantics() {
        let error = parse_control_session_id(SessionRouteParams::new("not-a-session")).unwrap_err();
        assert_eq!(error.kind(), SessionControlRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid session id");
    }
}
