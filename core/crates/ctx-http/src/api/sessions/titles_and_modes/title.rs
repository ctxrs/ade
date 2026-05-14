use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct GenerateSessionTitleReq {
    #[serde(default)]
    pub(crate) prompt: Option<String>,
    #[serde(default)]
    pub(crate) force: Option<bool>,
}

pub(crate) async fn generate_session_title(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Json(req): Json<GenerateSessionTitleReq>,
) -> Result<Json<Session>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    state
        .generate_session_title_for_request(session_id, req.prompt, req.force)
        .await
        .map(Json)
        .map_err(map_generate_session_title_error)
}

fn map_generate_session_title_error(
    error: crate::daemon::sessions::GenerateSessionTitleError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        crate::daemon::sessions::GenerateSessionTitleError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ),
        crate::daemon::sessions::GenerateSessionTitleError::PromptRequired => (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "prompt required".to_string(),
            }),
        ),
        crate::daemon::sessions::GenerateSessionTitleError::Skipped => (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "title generation skipped".to_string(),
            }),
        ),
        crate::daemon::sessions::GenerateSessionTitleError::Internal(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error.to_string()),
            }),
        ),
    }
}
