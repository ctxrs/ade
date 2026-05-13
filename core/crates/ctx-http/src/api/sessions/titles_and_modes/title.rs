use super::*;
use crate::daemon::sessions::title_generation::{
    configured_title_generation_settings, maybe_generate_session_title,
};

#[derive(Debug, Deserialize)]
pub(crate) struct GenerateSessionTitleReq {
    #[serde(default)]
    pub(crate) prompt: Option<String>,
    #[serde(default)]
    pub(crate) force: Option<bool>,
}

pub(crate) async fn generate_session_title(
    State(state): State<Arc<AppState>>,
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

    let store = store_for_existing_session_api_error(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    let prompt = if let Some(prompt) = req
        .prompt
        .as_ref()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
    {
        prompt
    } else {
        store
            .get_first_user_message_content(session_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?
            .filter(|p| !p.trim().is_empty())
            .ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "prompt required".to_string(),
                }),
            ))?
    };

    let force = req.force.unwrap_or(true);
    let cfg = configured_title_generation_settings(&state).await;
    maybe_generate_session_title(state.clone(), session.clone(), prompt, force, cfg)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "title generation skipped".to_string(),
            }),
        ))?;

    let store = store_for_existing_session_api_error(&state, session_id).await?;
    let updated = store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    Ok(Json(updated))
}
