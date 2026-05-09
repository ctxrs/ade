use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use ctx_core::ids::{MessageId, RunId, SessionId, TurnId};
use ctx_core::models::{RunStatus, SessionEventType};
use ctx_providers::adapters::{ProviderTurnOutcome, ProviderTurnStatus};

use crate::daemon::AppState;

use super::persistence::persist_terminal_events;
use super::types::{FailedTurnTerminalization, InterruptedTurnTerminalization};

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
        RunStatus::Completed,
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
    interruption: InterruptedTurnTerminalization<'_>,
) -> Result<()> {
    let mut events = Vec::new();
    if interruption.emit_interrupt_event {
        events.push((
            SessionEventType::TurnInterrupted,
            json!({
                "reason": interruption.reason,
                "provider_cancelled": interruption.provider_cancelled,
                "status": "interrupted",
            }),
        ));
    }
    events.push((
        SessionEventType::TurnFinished,
        json!({
            "message_id": message_id.0,
            "status": "interrupted",
            "reason": interruption.reason,
            "provider_cancelled": interruption.provider_cancelled,
        }),
    ));

    persist_terminal_events(
        state,
        session_id,
        run_id,
        turn_id,
        RunStatus::Cancelled,
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
    failure: FailedTurnTerminalization<'_>,
) -> Result<()> {
    let mut finished = json!({
        "message_id": message_id.0,
        "status": "failed",
    });
    if let Some(obj) = finished.as_object_mut() {
        if let Some(reason) = failure.reason {
            obj.insert("reason".to_string(), json!(reason));
        }
        obj.insert("message".to_string(), json!(failure.message));
        if let Some(details) = failure.details {
            obj.insert("details".to_string(), details);
        }
        if let Some(kind) = failure.kind {
            obj.insert("kind".to_string(), kind);
        }
    }

    persist_terminal_events(
        state,
        session_id,
        run_id,
        turn_id,
        RunStatus::Failed,
        &[
            SessionEventType::AssistantChunk,
            SessionEventType::ThoughtChunk,
            SessionEventType::ContextWindowUpdate,
        ],
        vec![(SessionEventType::TurnFinished, finished)],
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
                InterruptedTurnTerminalization {
                    reason: outcome.reason.as_deref().unwrap_or("interrupted"),
                    provider_cancelled: outcome.provider_cancelled.unwrap_or(false),
                    emit_interrupt_event: !outcome.terminal_event_emitted,
                },
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
                FailedTurnTerminalization {
                    message: outcome.message.as_deref().unwrap_or("provider turn failed"),
                    reason: outcome.reason.as_deref(),
                    details: outcome.details,
                    kind: outcome.kind,
                },
            )
            .await
        }
    }
}
