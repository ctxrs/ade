#[path = "seed_turn/events.rs"]
mod events;
#[path = "seed_turn/materialize.rs"]
mod materialize;
#[path = "seed_turn/records.rs"]
mod records;
#[cfg(test)]
#[path = "seed_turn/tests.rs"]
mod tests;

use axum::http::StatusCode;
use axum::Json;
use chrono::{DateTime, Utc};

use super::types::SeedTranscriptTurnReq;
use crate::api::errors::ApiErrorResp;
use ctx_core::ids::{SessionId, TaskId};
use ctx_store::Store;
use events::{
    append_assistant_message_event, append_done_event, append_turn_finished_event,
    append_turn_started_event, append_user_message_event,
};
use materialize::{insert_assistant_message, insert_session_turn, insert_user_message};
use records::SeededTurn;

type SeedTurnResult<T> = Result<T, (StatusCode, Json<ApiErrorResp>)>;

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
) -> SeedTurnResult<SeedTurnCounts> {
    let seeded = SeededTurn::new(session_id, task_id, index, base_time, turn);
    let mut messages = 0usize;
    let mut events = 0usize;

    if materialize_turn {
        insert_user_message(store, &seeded).await?;
        messages += 1;
    }

    let user_event = append_user_message_event(store, &seeded).await?;
    events += 1;

    append_turn_started_event(store, &seeded).await?;
    events += 1;

    if materialize_turn {
        insert_assistant_message(store, &seeded).await?;
        messages += 1;
    }

    append_assistant_message_event(store, &seeded).await?;
    events += 1;

    let done_event = append_done_event(store, &seeded, turn.context_window.as_ref()).await?;
    events += 1;

    append_turn_finished_event(store, &seeded).await?;
    events += 1;

    if materialize_turn {
        insert_session_turn(
            store,
            &seeded,
            turn.context_window.clone(),
            user_event.seq,
            done_event.seq,
        )
        .await?;
    }

    Ok(SeedTurnCounts { messages, events })
}

fn internal_error(message: &'static str) -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResp {
            error: message.to_string(),
        }),
    )
}
