use std::sync::Arc;

use axum::http::StatusCode;
use axum::Json;

use super::super::types::SeedTranscriptReq;
use crate::api::errors::ApiErrorResp;
use crate::daemon::AppState;
use ctx_core::ids::SessionId;
use ctx_core::models::Session;

pub(super) async fn apply_seed_transcript_metadata(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    session_id: SessionId,
    session: Session,
    req: &SeedTranscriptReq,
) -> Result<Session, (StatusCode, Json<ApiErrorResp>)> {
    if let Some(session_title) = req
        .session_title
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        store
            .update_session_title(session_id, session_title.to_string())
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to update session title".to_string(),
                    }),
                )
            })?;
    }

    if let Some(task_title) = req
        .task_title
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        store
            .update_task_title(session.task_id, task_title.to_string())
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to update task title".to_string(),
                    }),
                )
            })?;
    }

    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "failed to reload session".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;
    state.sessions.remember_session_meta(&session).await;
    Ok(session)
}
