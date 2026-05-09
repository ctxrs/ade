use super::super::*;

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

    // Validate the session exists (prevents accidentally fulfilling a prompt for a deleted session).
    let store = store_for_existing_session_api_error(&state, session_id).await?;
    let exists = store
        .get_session(session_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to load session".to_string(),
                }),
            )
        })?
        .is_some();
    if !exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ));
    }

    let tool_call_id = req.tool_call_id.trim().to_string();
    if tool_call_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "missing tool_call_id".to_string(),
            }),
        ));
    }

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
    let answers = req.answers.unwrap_or_default();
    let answers_for_event = answers.clone();

    let ok = state
        .core
        .ask_user_question
        .submit(
            &session_uuid.to_string(),
            &tool_call_id,
            AskUserQuestionAnswer { outcome, answers },
        )
        .await;

    if !ok {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "no pending AskUserQuestion for this tool_call_id".to_string(),
            }),
        ));
    }

    if let Ok(store) = state.store_for_session(session_id).await {
        if let Ok(event) = store
            .append_session_event(
                session_id,
                None,
                None,
                SessionEventType::Notice,
                serde_json::json!({
                    "kind": "ask_user_question_answered",
                    "tool_call_id": tool_call_id,
                    "outcome": outcome.as_str(),
                    "answers": answers_for_event,
                }),
            )
            .await
        {
            state.publish_event(event).await;
        }
    }

    Ok(Json(SubmitAskUserQuestionResp { ok: true }))
}
