use serde::{Deserialize, Serialize};

use super::route_contract::parse_session_route_id;
use crate::daemon::sessions::{
    DemoSeedTranscript, DemoSeedTranscriptError, DemoSeedTranscriptTurn,
};
use crate::daemon::{SessionRouteParams, SessionsHandle};

#[derive(Debug, Clone, Deserialize)]
pub struct DemoSeedTranscriptRouteTurn {
    user: String,
    assistant: String,
    #[serde(default)]
    context_window: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DemoSeedTranscriptRouteRequest {
    #[serde(default)]
    session_title: Option<String>,
    #[serde(default)]
    task_title: Option<String>,
    #[serde(default)]
    append: bool,
    #[serde(default = "default_demo_seed_transcript_refresh")]
    refresh: bool,
    #[serde(default)]
    materialize_tail_turns: Option<usize>,
    turns: Vec<DemoSeedTranscriptRouteTurn>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoSeedTranscriptRouteResponse {
    pub session_id: String,
    pub seeded_turns: usize,
    pub seeded_messages: usize,
    pub seeded_events: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DemoSeedTranscriptRouteErrorKind {
    BadRequest,
    NotFound,
    Conflict,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DemoSeedTranscriptRouteError {
    kind: DemoSeedTranscriptRouteErrorKind,
    message: &'static str,
}

impl DemoSeedTranscriptRouteError {
    fn new(kind: DemoSeedTranscriptRouteErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    fn bad_request(message: &'static str) -> Self {
        Self::new(DemoSeedTranscriptRouteErrorKind::BadRequest, message)
    }

    fn not_found(message: &'static str) -> Self {
        Self::new(DemoSeedTranscriptRouteErrorKind::NotFound, message)
    }

    fn conflict(message: &'static str) -> Self {
        Self::new(DemoSeedTranscriptRouteErrorKind::Conflict, message)
    }

    fn internal(message: &'static str) -> Self {
        Self::new(DemoSeedTranscriptRouteErrorKind::Internal, message)
    }

    pub fn kind(&self) -> DemoSeedTranscriptRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &'static str {
        self.message
    }
}

impl SessionsHandle {
    pub async fn seed_demo_transcript_for_route(
        &self,
        params: SessionRouteParams,
        request: DemoSeedTranscriptRouteRequest,
    ) -> Result<DemoSeedTranscriptRouteResponse, DemoSeedTranscriptRouteError> {
        let session_id = parse_session_route_id(params.session_id())
            .map_err(|_| DemoSeedTranscriptRouteError::bad_request("invalid session id"))?;
        let seed = request.into_seed()?;
        let result = self
            .seed_demo_transcript(session_id, seed)
            .await
            .map_err(demo_seed_transcript_route_error)?;
        Ok(DemoSeedTranscriptRouteResponse {
            session_id: session_id.0.to_string(),
            seeded_turns: result.seeded_turns,
            seeded_messages: result.seeded_messages,
            seeded_events: result.seeded_events,
        })
    }
}

impl DemoSeedTranscriptRouteRequest {
    fn into_seed(self) -> Result<DemoSeedTranscript, DemoSeedTranscriptRouteError> {
        if self.turns.is_empty() {
            return Err(DemoSeedTranscriptRouteError::bad_request(
                "turns must not be empty",
            ));
        }
        Ok(DemoSeedTranscript {
            session_title: self.session_title,
            task_title: self.task_title,
            append: self.append,
            refresh: self.refresh,
            materialize_tail_turns: self.materialize_tail_turns,
            turns: self
                .turns
                .into_iter()
                .map(|turn| DemoSeedTranscriptTurn {
                    user: turn.user,
                    assistant: turn.assistant,
                    context_window: turn.context_window,
                })
                .collect(),
        })
    }
}

fn demo_seed_transcript_route_error(
    error: DemoSeedTranscriptError,
) -> DemoSeedTranscriptRouteError {
    match error {
        DemoSeedTranscriptError::SessionNotFound => {
            DemoSeedTranscriptRouteError::not_found("session not found")
        }
        DemoSeedTranscriptError::SessionAlreadyHasMessages => {
            DemoSeedTranscriptRouteError::conflict(
                "session already has messages; seed into a fresh session",
            )
        }
        DemoSeedTranscriptError::StoreUnavailable => {
            DemoSeedTranscriptRouteError::internal("failed to load session")
        }
        DemoSeedTranscriptError::InspectMessages => {
            DemoSeedTranscriptRouteError::internal("failed to inspect session messages")
        }
        DemoSeedTranscriptError::UpdateSessionTitle => {
            DemoSeedTranscriptRouteError::internal("failed to update session title")
        }
        DemoSeedTranscriptError::UpdateTaskTitle => {
            DemoSeedTranscriptRouteError::internal("failed to update task title")
        }
        DemoSeedTranscriptError::ReloadSession => {
            DemoSeedTranscriptRouteError::internal("failed to reload session")
        }
        DemoSeedTranscriptError::InsertUserMessage => {
            DemoSeedTranscriptRouteError::internal("failed to insert user message")
        }
        DemoSeedTranscriptError::InsertAssistantMessage => {
            DemoSeedTranscriptRouteError::internal("failed to insert assistant message")
        }
        DemoSeedTranscriptError::InsertSessionTurn => {
            DemoSeedTranscriptRouteError::internal("failed to insert session turn")
        }
        DemoSeedTranscriptError::AppendUserEvent => {
            DemoSeedTranscriptRouteError::internal("failed to append user event")
        }
        DemoSeedTranscriptError::AppendTurnStartedEvent => {
            DemoSeedTranscriptRouteError::internal("failed to append turn started event")
        }
        DemoSeedTranscriptError::AppendAssistantEvent => {
            DemoSeedTranscriptRouteError::internal("failed to append assistant event")
        }
        DemoSeedTranscriptError::AppendDoneEvent => {
            DemoSeedTranscriptRouteError::internal("failed to append done event")
        }
        DemoSeedTranscriptError::AppendTurnFinishedEvent => {
            DemoSeedTranscriptRouteError::internal("failed to append turn finished event")
        }
    }
}

fn default_demo_seed_transcript_refresh() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_request_requires_turns() {
        let request = DemoSeedTranscriptRouteRequest {
            session_title: None,
            task_title: None,
            append: false,
            refresh: true,
            materialize_tail_turns: None,
            turns: Vec::new(),
        };

        let error = match request.into_seed() {
            Ok(_) => panic!("empty seed transcript request should fail"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), DemoSeedTranscriptRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "turns must not be empty");
    }

    #[test]
    fn route_error_maps_domain_errors() {
        let error = demo_seed_transcript_route_error(DemoSeedTranscriptError::SessionNotFound);
        assert_eq!(error.kind(), DemoSeedTranscriptRouteErrorKind::NotFound);
        assert_eq!(error.message(), "session not found");

        let error =
            demo_seed_transcript_route_error(DemoSeedTranscriptError::SessionAlreadyHasMessages);
        assert_eq!(error.kind(), DemoSeedTranscriptRouteErrorKind::Conflict);
        assert_eq!(
            error.message(),
            "session already has messages; seed into a fresh session"
        );
    }
}
