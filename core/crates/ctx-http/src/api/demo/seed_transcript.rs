use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{Duration, Utc};

use super::dev_mode::dev_tools_enabled;
use super::seed_turn::seed_transcript_turn;
use super::types::{SeedTranscriptReq, SeedTranscriptResp};
use crate::api::errors::ApiErrorResp;
use crate::daemon::AppState;
use ctx_core::ids::SessionId;

#[path = "seed_transcript/metadata.rs"]
mod metadata;

pub(crate) async fn dev_seed_session_transcript(
    State(state): State<Arc<AppState>>,
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

    let store = state.store_for_session(session_id).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        )
    })?;
    let session = store
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
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    if !req.append
        && !store
            .list_messages_for_session(session_id)
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to inspect session messages".to_string(),
                    }),
                )
            })?
            .is_empty()
    {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "session already has messages; seed into a fresh session".to_string(),
            }),
        ));
    }

    let session =
        metadata::apply_seed_transcript_metadata(&state, &store, session_id, session, &req).await?;

    let mut seeded_messages = 0usize;
    let mut seeded_events = 0usize;
    let base_time = Utc::now() - Duration::minutes(req.turns.len() as i64);

    let materialize_from_index = req
        .materialize_tail_turns
        .map(|tail| req.turns.len().saturating_sub(tail));

    for (index, turn) in req.turns.iter().enumerate() {
        let materialize_turn = materialize_from_index
            .map(|from_index| index >= from_index)
            .unwrap_or(true);
        let counts = seed_transcript_turn(
            &store,
            session_id,
            session.task_id,
            index,
            base_time,
            turn,
            materialize_turn,
        )
        .await?;
        seeded_messages += counts.messages;
        seeded_events += counts.events;
    }

    if req.refresh {
        state.refresh_session_head_cache(session_id).await;

        if let Err(err) = state.emit_workspace_task_upsert(session.task_id).await {
            tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed after demo transcript seed: {err:?}");
        }
    }

    Ok(Json(SeedTranscriptResp {
        session_id: session_id.0.to_string(),
        seeded_turns: req.turns.len(),
        seeded_messages,
        seeded_events,
    }))
}
