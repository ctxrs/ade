use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use super::errors::ApiErrorResp;
use crate::daemon::AppState;
use ctx_core::ids::{MessageId, RunId, SessionId, TurnId};
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, SessionEventType, SessionTurn, SessionTurnStatus,
};

fn dev_tools_enabled() -> bool {
    std::env::var("CTX_DEV_MODE")
        .ok()
        .as_deref()
        .and_then(ctx_core::boolish::parse_boolish)
        .unwrap_or(false)
}

#[derive(Debug, Deserialize)]
pub(crate) struct SeedTranscriptTurnReq {
    pub(crate) user: String,
    pub(crate) assistant: String,
    #[serde(default)]
    pub(crate) context_window: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SeedTranscriptReq {
    #[serde(default)]
    pub(crate) session_title: Option<String>,
    #[serde(default)]
    pub(crate) task_title: Option<String>,
    pub(crate) turns: Vec<SeedTranscriptTurnReq>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SeedTranscriptResp {
    pub(crate) session_id: String,
    pub(crate) seeded_turns: usize,
    pub(crate) seeded_messages: usize,
    pub(crate) seeded_events: usize,
}

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

    if !store
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

    let mut seeded_messages = 0usize;
    let mut seeded_events = 0usize;
    let base_time = Utc::now() - Duration::minutes(req.turns.len() as i64);

    for (index, turn) in req.turns.iter().enumerate() {
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let user_message_id = MessageId::new();
        let assistant_message_id = MessageId::new();
        let user_created_at = base_time + Duration::seconds((index as i64) * 12);
        let assistant_created_at = user_created_at + Duration::seconds(4);

        let user_message = Message {
            id: user_message_id,
            session_id,
            task_id: session.task_id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
            turn_sequence: Some(0),
            order_seq: Some((index as i64) * 2 + 1),
            role: MessageRole::User,
            content: turn.user.clone(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Immediate,
            delivered_at: Some(user_created_at),
            created_at: user_created_at,
        };
        let assistant_message = Message {
            id: assistant_message_id,
            session_id,
            task_id: session.task_id,
            run_id: Some(run_id),
            turn_id: Some(turn_id),
            turn_sequence: Some(1),
            order_seq: Some((index as i64) * 2 + 2),
            role: MessageRole::Assistant,
            content: turn.assistant.clone(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Immediate,
            delivered_at: Some(assistant_created_at),
            created_at: assistant_created_at,
        };

        store
            .insert_message(user_message.clone())
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to insert user message".to_string(),
                    }),
                )
            })?;
        seeded_messages += 1;

        let user_event = store
            .append_session_event(
                session_id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::UserMessage,
                serde_json::json!({
                    "message_id": user_message.id.0,
                    "content": user_message.content,
                    "delivery": user_message.delivery,
                    "attachments": [],
                    "order_seq": (index as i64) * 2 + 1,
                }),
            )
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to append user event".to_string(),
                    }),
                )
            })?;
        seeded_events += 1;

        store
            .append_session_event(
                session_id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::TurnStarted,
                serde_json::json!({
                    "message_id": user_message.id.0,
                }),
            )
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to append turn started event".to_string(),
                    }),
                )
            })?;
        seeded_events += 1;

        store
            .insert_message(assistant_message.clone())
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to insert assistant message".to_string(),
                    }),
                )
            })?;
        seeded_messages += 1;

        store
            .append_session_event(
                session_id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::AssistantMessageInserted,
                serde_json::json!({
                    "message_id": assistant_message.id.0,
                    "content": assistant_message.content,
                    "attachments": [],
                    "delivery": assistant_message.delivery,
                    "order_seq": (index as i64) * 2 + 2,
                    "turn_sequence": 1,
                }),
            )
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to append assistant event".to_string(),
                    }),
                )
            })?;
        seeded_events += 1;

        let done_event = store
            .append_session_event(
                session_id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::Done,
                {
                    let mut payload = serde_json::json!({
                        "status": "completed",
                    });
                    if let Some(metrics) = turn.context_window.clone() {
                        if let Some(obj) = payload.as_object_mut() {
                            obj.insert("context_window".to_string(), metrics);
                        }
                    }
                    payload
                },
            )
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to append done event".to_string(),
                    }),
                )
            })?;
        seeded_events += 1;

        store
            .append_session_event(
                session_id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::TurnFinished,
                serde_json::json!({
                    "message_id": user_message.id.0,
                    "status": "completed",
                }),
            )
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to append turn finished event".to_string(),
                    }),
                )
            })?;
        seeded_events += 1;

        store
            .insert_session_turn(SessionTurn {
                turn_id,
                session_id,
                run_id: Some(run_id),
                user_message_id: Some(user_message_id),
                status: SessionTurnStatus::Completed,
                start_seq: Some(user_event.seq),
                end_seq: Some(done_event.seq),
                started_at: user_created_at,
                updated_at: assistant_created_at,
                assistant_partial: None,
                thought_partial: None,
                metrics_json: turn.context_window.clone(),
                tool_total: 0,
                tool_pending: 0,
                tool_running: 0,
                tool_completed: 0,
                tool_failed: 0,
            })
            .await
            .map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: "failed to insert session turn".to_string(),
                    }),
                )
            })?;
    }

    state.refresh_session_head_cache(session_id).await;

    if let Err(err) = state.emit_workspace_task_upsert(session.task_id).await {
        tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed after demo transcript seed: {err:?}");
    }

    Ok(Json(SeedTranscriptResp {
        session_id: session_id.0.to_string(),
        seeded_turns: req.turns.len(),
        seeded_messages,
        seeded_events,
    }))
}
