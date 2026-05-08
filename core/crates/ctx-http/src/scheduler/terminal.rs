use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::{json, Value};

use ctx_core::ids::{MessageId, RunId, SessionId, TurnId};
use ctx_core::models::{RunStatus, SessionEvent, SessionEventType};
use ctx_providers::adapters::{ProviderTurnOutcome, ProviderTurnStatus};

use crate::daemon::AppState;

use super::persistence::{
    is_transient_store_error, STORE_WRITE_RETRY_BASE_MS, STORE_WRITE_RETRY_LIMIT,
};

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

async fn persist_turn_terminal_events_with_retry(
    store: &ctx_store::Store,
    session_id: SessionId,
    run_id: Option<RunId>,
    turn_id: TurnId,
    events: &[(SessionEventType, Value)],
) -> Result<Vec<SessionEvent>> {
    let mut attempt = 0usize;
    loop {
        match store
            .persist_turn_terminal_events(session_id, run_id, turn_id, events.to_vec())
            .await
        {
            Ok(persisted) => return Ok(persisted),
            Err(err) => {
                if !is_transient_store_error(&err) || attempt >= STORE_WRITE_RETRY_LIMIT {
                    return Err(err);
                }
                attempt += 1;
                let backoff_ms = STORE_WRITE_RETRY_BASE_MS.saturating_mul(attempt as u64);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
        }
    }
}

async fn persist_terminal_events(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: Option<RunId>,
    turn_id: TurnId,
    run_status: RunStatus,
    cleanup_types: &[SessionEventType],
    events: Vec<(SessionEventType, Value)>,
) -> Result<()> {
    let store = state.store_for_session(session_id).await?;
    cleanup_turn_stream_state(&store, session_id, turn_id, cleanup_types).await;
    let persisted =
        persist_turn_terminal_events_with_retry(&store, session_id, run_id, turn_id, &events)
            .await?;
    ctx_org_policy::admission::update_run_terminal_status(&store, run_id, run_status).await;
    publish_persisted_events(state, persisted).await;
    Ok(())
}

pub(crate) struct InterruptedTurnTerminalization<'a> {
    pub(crate) reason: &'a str,
    pub(crate) provider_cancelled: bool,
    pub(crate) emit_interrupt_event: bool,
}

pub(crate) struct FailedTurnTerminalization<'a> {
    pub(crate) message: &'a str,
    pub(crate) reason: Option<&'a str>,
    pub(crate) details: Option<Value>,
    pub(crate) kind: Option<Value>,
    pub(crate) emit_error_event: bool,
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
    let mut events = Vec::new();
    if failure.emit_error_event {
        let mut payload = json!({
            "message_id": message_id.0,
            "message": failure.message,
            "error": failure.message,
            "status": "failed",
        });
        if let Some(obj) = payload.as_object_mut() {
            if let Some(reason) = failure.reason {
                obj.insert("reason".to_string(), json!(reason));
            }
            if let Some(details) = failure.details.clone() {
                obj.insert("details".to_string(), details);
            }
            if let Some(kind) = failure.kind.clone() {
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
    events.push((SessionEventType::TurnFinished, finished));

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
                    emit_error_event: !outcome.terminal_event_emitted,
                },
            )
            .await
        }
    }
}
