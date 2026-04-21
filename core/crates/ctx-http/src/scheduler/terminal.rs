use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};

use ctx_core::ids::{MessageId, RunId, SessionId, TurnId};
use ctx_core::models::SessionEventType;
use ctx_providers::adapters::{ProviderTurnOutcome, ProviderTurnStatus};

use crate::daemon::AppState;

async fn publish_persisted_events(
    state: &Arc<AppState>,
    events: Vec<ctx_core::models::SessionEvent>,
) {
    for event in events {
        state.publish_event(event).await;
    }
}

async fn cleanup_turn_stream_state(
    store: &ctx_store::Store,
    session_id: SessionId,
    turn_id: TurnId,
    event_types: &[SessionEventType],
) {
    if event_types.is_empty() {
        return;
    }
    if let Err(err) = store
        .delete_session_events_for_turn_types(session_id, turn_id, event_types)
        .await
    {
        tracing::warn!(
            session_id = %session_id.0,
            turn_id = %turn_id.0,
            "failed to delete transient turn events before terminalization: {err:#}"
        );
    }
}

async fn persist_terminal_events(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: Option<RunId>,
    turn_id: TurnId,
    cleanup_types: &[SessionEventType],
    events: Vec<(SessionEventType, Value)>,
) -> Result<()> {
    let store = state.store_for_session(session_id).await?;
    cleanup_turn_stream_state(&store, session_id, turn_id, cleanup_types).await;
    let persisted = store
        .persist_turn_terminal_events(session_id, run_id, turn_id, events)
        .await?;
    publish_persisted_events(state, persisted).await;
    Ok(())
}

pub(crate) async fn finalize_completed_turn(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: Option<RunId>,
    turn_id: TurnId,
    message_id: MessageId,
) -> Result<()> {
    persist_terminal_events(
        state,
        session_id,
        run_id,
        turn_id,
        &[
            SessionEventType::ThoughtChunk,
            SessionEventType::ContextWindowUpdate,
        ],
        vec![(
            SessionEventType::TurnFinished,
            json!({
                "message_id": message_id.0,
                "status": "completed",
            }),
        )],
    )
    .await
}

pub(crate) async fn finalize_interrupted_turn(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: Option<RunId>,
    turn_id: TurnId,
    message_id: MessageId,
    reason: &str,
    provider_cancelled: bool,
    emit_interrupt_event: bool,
) -> Result<()> {
    let mut events = Vec::new();
    if emit_interrupt_event {
        events.push((
            SessionEventType::TurnInterrupted,
            json!({
                "reason": reason,
                "provider_cancelled": provider_cancelled,
                "status": "interrupted",
            }),
        ));
    }
    events.push((
        SessionEventType::TurnFinished,
        json!({
            "message_id": message_id.0,
            "status": "interrupted",
            "reason": reason,
            "provider_cancelled": provider_cancelled,
        }),
    ));

    persist_terminal_events(
        state,
        session_id,
        run_id,
        turn_id,
        &[
            SessionEventType::AssistantChunk,
            SessionEventType::ThoughtChunk,
            SessionEventType::ContextWindowUpdate,
        ],
        events,
    )
    .await
}

pub(crate) async fn finalize_failed_turn(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: Option<RunId>,
    turn_id: TurnId,
    message_id: MessageId,
    message: &str,
    reason: Option<&str>,
    details: Option<Value>,
    kind: Option<Value>,
    emit_error_event: bool,
) -> Result<()> {
    let mut events = Vec::new();
    if emit_error_event {
        let mut payload = json!({
            "message_id": message_id.0,
            "message": message,
            "error": message,
            "status": "failed",
        });
        if let Some(obj) = payload.as_object_mut() {
            if let Some(reason) = reason {
                obj.insert("reason".to_string(), json!(reason));
            }
            if let Some(details) = details.clone() {
                obj.insert("details".to_string(), details);
            }
            if let Some(kind) = kind.clone() {
                obj.insert("kind".to_string(), kind);
            }
        }
        events.push((SessionEventType::Error, payload));
    }

    let mut finished = json!({
        "message_id": message_id.0,
        "status": "failed",
    });
    if let Some(obj) = finished.as_object_mut() {
        if let Some(reason) = reason {
            obj.insert("reason".to_string(), json!(reason));
        }
        obj.insert("message".to_string(), json!(message));
        if let Some(details) = details {
            obj.insert("details".to_string(), details);
        }
        if let Some(kind) = kind {
            obj.insert("kind".to_string(), kind);
        }
    }
    events.push((SessionEventType::TurnFinished, finished));

    persist_terminal_events(
        state,
        session_id,
        run_id,
        turn_id,
        &[
            SessionEventType::AssistantChunk,
            SessionEventType::ThoughtChunk,
            SessionEventType::ContextWindowUpdate,
        ],
        events,
    )
    .await
}

pub(crate) async fn finalize_provider_outcome(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: Option<RunId>,
    turn_id: TurnId,
    message_id: MessageId,
    outcome: ProviderTurnOutcome,
) -> Result<()> {
    match outcome.status {
        ProviderTurnStatus::Completed => {
            finalize_completed_turn(state, session_id, run_id, turn_id, message_id).await
        }
        ProviderTurnStatus::Interrupted => {
            finalize_interrupted_turn(
                state,
                session_id,
                run_id,
                turn_id,
                message_id,
                outcome.reason.as_deref().unwrap_or("interrupted"),
                outcome.provider_cancelled.unwrap_or(false),
                !outcome.terminal_event_emitted,
            )
            .await
        }
        ProviderTurnStatus::Failed => {
            finalize_failed_turn(
                state,
                session_id,
                run_id,
                turn_id,
                message_id,
                outcome.message.as_deref().unwrap_or("provider turn failed"),
                outcome.reason.as_deref(),
                outcome.details,
                outcome.kind,
                !outcome.terminal_event_emitted,
            )
            .await
        }
    }
}
