use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use super::dev_mode::dev_tools_enabled;
use super::types::{SeedTranscriptReq, SeedTranscriptResp};
use crate::api::errors::ApiErrorResp;
use crate::daemon::sessions::{
    DemoSeedTranscript, DemoSeedTranscriptError, DemoSeedTranscriptTurn,
};
use crate::daemon::SessionsHandle;
use ctx_core::ids::SessionId;

pub(crate) async fn dev_seed_session_transcript(
    State(sessions): State<SessionsHandle>,
    Path(id): Path<String>,
    Json(req): Json<SeedTranscriptReq>,
) -> Result<Json<SeedTranscriptResp>, (StatusCode, Json<ApiErrorResp>)> {
    if !dev_tools_enabled() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "dev tools are disabled".to_string(),
            }),
        ));
    }

    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    if req.turns.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "turns must not be empty".to_string(),
            }),
        ));
    }

    let seed = DemoSeedTranscript {
        session_title: req.session_title,
        task_title: req.task_title,
        append: req.append,
        refresh: req.refresh,
        materialize_tail_turns: req.materialize_tail_turns,
        turns: req
            .turns
            .into_iter()
            .map(|turn| DemoSeedTranscriptTurn {
                user: turn.user,
                assistant: turn.assistant,
                context_window: turn.context_window,
            })
            .collect(),
    };
    let result = sessions
        .seed_demo_transcript(session_id, seed)
        .await
        .map_err(seed_transcript_error)?;

    Ok(Json(SeedTranscriptResp {
        session_id: session_id.0.to_string(),
        seeded_turns: result.seeded_turns,
        seeded_messages: result.seeded_messages,
        seeded_events: result.seeded_events,
    }))
}

fn seed_transcript_error(error: DemoSeedTranscriptError) -> (StatusCode, Json<ApiErrorResp>) {
    let (status, message) = match error {
        DemoSeedTranscriptError::SessionNotFound => (StatusCode::NOT_FOUND, "session not found"),
        DemoSeedTranscriptError::SessionAlreadyHasMessages => (
            StatusCode::CONFLICT,
            "session already has messages; seed into a fresh session",
        ),
        DemoSeedTranscriptError::StoreUnavailable => {
            (StatusCode::INTERNAL_SERVER_ERROR, "failed to load session")
        }
        DemoSeedTranscriptError::InspectMessages => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to inspect session messages",
        ),
        DemoSeedTranscriptError::UpdateSessionTitle => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to update session title",
        ),
        DemoSeedTranscriptError::UpdateTaskTitle => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to update task title",
        ),
        DemoSeedTranscriptError::ReloadSession => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to reload session",
        ),
        DemoSeedTranscriptError::InsertUserMessage => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to insert user message",
        ),
        DemoSeedTranscriptError::InsertAssistantMessage => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to insert assistant message",
        ),
        DemoSeedTranscriptError::InsertSessionTurn => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to insert session turn",
        ),
        DemoSeedTranscriptError::AppendUserEvent => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to append user event",
        ),
        DemoSeedTranscriptError::AppendTurnStartedEvent => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to append turn started event",
        ),
        DemoSeedTranscriptError::AppendAssistantEvent => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to append assistant event",
        ),
        DemoSeedTranscriptError::AppendDoneEvent => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to append done event",
        ),
        DemoSeedTranscriptError::AppendTurnFinishedEvent => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to append turn finished event",
        ),
    };
    (
        status,
        Json(ApiErrorResp {
            error: message.to_string(),
        }),
    )
}
