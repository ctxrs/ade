use super::super::*;
use crate::daemon::sessions::ask_user::{
    submit_ask_user_answer, SubmitAskUserAnswer, SubmitAskUserAnswerError,
};
use ctx_observability::logs;
use ctx_providers::ask_user_question::AskUserQuestionOutcome;

#[derive(Debug, Deserialize)]
pub(crate) struct SubmitAskUserQuestionReq {
    tool_call_id: String,
    #[serde(default)]
    outcome: Option<String>,
    #[serde(default)]
    answers: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SubmitAskUserQuestionResp {
    ok: bool,
}

pub(crate) async fn submit_ask_user_question(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SubmitAskUserQuestionReq>,
) -> Result<Json<SubmitAskUserQuestionResp>, (StatusCode, Json<ApiErrorResp>)> {
    let session_uuid = uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?;
    let session_id = SessionId(session_uuid);

    let outcome = match req.outcome.as_deref() {
        Some("cancelled") => AskUserQuestionOutcome::Cancelled,
        Some("submitted") | None => AskUserQuestionOutcome::Submitted,
        Some(other) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("invalid outcome: {other}"),
                }),
            ));
        }
    };

    submit_ask_user_answer(
        &state,
        session_id,
        SubmitAskUserAnswer {
            tool_call_id: req.tool_call_id,
            outcome,
            answers: req.answers.unwrap_or_default(),
        },
    )
    .await
    .map_err(submit_ask_user_answer_error)?;

    Ok(Json(SubmitAskUserQuestionResp { ok: true }))
}

fn submit_ask_user_answer_error(
    error: SubmitAskUserAnswerError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        SubmitAskUserAnswerError::MissingToolCallId => {
            api_error(StatusCode::BAD_REQUEST, "missing tool_call_id".to_string())
        }
        SubmitAskUserAnswerError::SessionNotFound => {
            api_error(StatusCode::NOT_FOUND, "session not found".to_string())
        }
        SubmitAskUserAnswerError::StoreUnavailable(err) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            logs::redact_sensitive(&err.to_string()),
        ),
        SubmitAskUserAnswerError::LoadSession => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to load session".to_string(),
        ),
        SubmitAskUserAnswerError::NoPendingQuestion => api_error(
            StatusCode::CONFLICT,
            "no pending AskUserQuestion for this tool_call_id".to_string(),
        ),
    }
}

fn api_error(status: StatusCode, error: String) -> (StatusCode, Json<ApiErrorResp>) {
    (status, Json(ApiErrorResp { error }))
}
