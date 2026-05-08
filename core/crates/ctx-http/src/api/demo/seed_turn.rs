use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Duration, Utc};

use super::types::SeedTranscriptTurnReq;
use crate::api::errors::ApiErrorResp;
use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId};
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, SessionEventType, SessionTurn, SessionTurnStatus,
};
use ctx_store::Store;

pub(super) struct SeedTurnCounts {
    pub(super) messages: usize,
    pub(super) events: usize,
}

pub(super) async fn seed_transcript_turn(
    store: &Store,
    session_id: SessionId,
    task_id: TaskId,
    index: usize,
    base_time: DateTime<Utc>,
    turn: &SeedTranscriptTurnReq,
    materialize_turn: bool,
) -> Result<SeedTurnCounts, (StatusCode, Json<ApiErrorResp>)> {
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let user_message_id = MessageId::new();
    let assistant_message_id = MessageId::new();
    let user_created_at = base_time + Duration::seconds((index as i64) * 12);
    let assistant_created_at = user_created_at + Duration::seconds(4);

    let user_message = Message {
        id: user_message_id,
        session_id,
        task_id,
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
        task_id,
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

    let mut messages = 0usize;
    let mut events = 0usize;

    if materialize_turn {
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
        messages += 1;
    }

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
    events += 1;

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
    events += 1;

    if materialize_turn {
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
        messages += 1;
    }

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
    events += 1;

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
    events += 1;

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
    events += 1;

    if materialize_turn {
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
                failure: None,
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

    Ok(SeedTurnCounts { messages, events })
}
