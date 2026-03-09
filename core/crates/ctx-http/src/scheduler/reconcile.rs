use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use ctx_core::ids::{RunId, TurnId};
use ctx_core::models::{SessionEvent, SessionEventType, SessionTurnStatus};

use crate::daemon::AppState;

use super::persistence::emit_event;

#[derive(Debug)]
struct ReconciledTerminalState<'a> {
    status: SessionTurnStatus,
    end_seq: Option<i64>,
    metrics: Option<&'a Value>,
    updated_at: DateTime<Utc>,
}

fn is_terminal_event(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::Done
            | SessionEventType::Error
            | SessionEventType::TurnInterrupted
            | SessionEventType::TurnFinished
    )
}

fn status_from_event(event: &SessionEvent) -> Option<SessionTurnStatus> {
    match event.event_type {
        SessionEventType::Done => Some(SessionTurnStatus::Completed),
        SessionEventType::Error => Some(SessionTurnStatus::Failed),
        SessionEventType::TurnInterrupted => Some(SessionTurnStatus::Interrupted),
        SessionEventType::TurnFinished => status_from_payload(&event.payload_json),
        _ => None,
    }
}

fn status_from_payload(payload: &Value) -> Option<SessionTurnStatus> {
    match payload.get("status").and_then(Value::as_str) {
        Some("completed") => Some(SessionTurnStatus::Completed),
        Some("failed" | "error") => Some(SessionTurnStatus::Failed),
        Some("interrupted") => Some(SessionTurnStatus::Interrupted),
        _ => None,
    }
}

fn latest_done_metrics(events: &[SessionEvent]) -> Option<&Value> {
    events
        .iter()
        .rev()
        .find(|event| matches!(event.event_type, SessionEventType::Done))
        .and_then(|event| event.payload_json.get("context_window"))
}

fn resolve_terminal_state(events: &[SessionEvent]) -> Option<ReconciledTerminalState<'_>> {
    let terminal_index = events
        .iter()
        .rposition(|event| is_terminal_event(&event.event_type))?;
    let event = &events[terminal_index];

    match event.event_type {
        SessionEventType::Done => Some(ReconciledTerminalState {
            status: SessionTurnStatus::Completed,
            end_seq: Some(event.seq),
            metrics: event.payload_json.get("context_window"),
            updated_at: event.created_at,
        }),
        SessionEventType::Error => Some(ReconciledTerminalState {
            status: SessionTurnStatus::Failed,
            end_seq: None,
            metrics: None,
            updated_at: event.created_at,
        }),
        SessionEventType::TurnInterrupted => Some(ReconciledTerminalState {
            status: SessionTurnStatus::Interrupted,
            end_seq: Some(event.seq),
            metrics: None,
            updated_at: event.created_at,
        }),
        SessionEventType::TurnFinished => {
            let status = status_from_payload(&event.payload_json).unwrap_or_else(|| {
                events[..terminal_index]
                    .iter()
                    .rev()
                    .find_map(status_from_event)
                    .unwrap_or(SessionTurnStatus::Completed)
            });
            let metrics = if status == SessionTurnStatus::Completed {
                latest_done_metrics(&events[..=terminal_index])
            } else {
                None
            };
            Some(ReconciledTerminalState {
                status,
                end_seq: Some(event.seq),
                metrics,
                updated_at: event.created_at,
            })
        }
        _ => None,
    }
}

pub async fn reconcile_turn_terminal_state(
    state: &Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
    run_id: Option<RunId>,
    turn_id: TurnId,
    fallback_reason: &str,
) -> Result<()> {
    let store = state.store_for_session(session_id).await?;
    let turn = store.get_session_turn(session_id, turn_id).await?;
    let Some(turn) = turn else {
        return Ok(());
    };
    if matches!(
        turn.status,
        SessionTurnStatus::Completed | SessionTurnStatus::Failed | SessionTurnStatus::Interrupted
    ) {
        return Ok(());
    }

    let events = store
        .list_session_events_for_turn(session_id, turn_id, false)
        .await?;
    if let Some(reconciled) = resolve_terminal_state(&events) {
        let _ = store
            .update_session_turn_status(
                session_id,
                turn_id,
                reconciled.status,
                reconciled.end_seq,
                reconciled.metrics,
                reconciled.updated_at,
            )
            .await;
        return Ok(());
    }

    let event = emit_event(
        state,
        session_id,
        run_id,
        Some(turn_id),
        SessionEventType::TurnInterrupted,
        json!({
            "reason": fallback_reason,
            "provider_cancelled": false,
            "status": "interrupted",
        }),
    )
    .await?;
    let _ = store
        .update_session_turn_status(
            session_id,
            turn_id,
            SessionTurnStatus::Interrupted,
            Some(event.seq),
            None,
            event.created_at,
        )
        .await;
    let _ = emit_event(
        state,
        session_id,
        run_id,
        Some(turn_id),
        SessionEventType::TurnFinished,
        json!({
            "message_id": turn.user_message_id.map(|id| id.0),
            "status": "interrupted",
            "reason": fallback_reason,
        }),
    )
    .await;
    Ok(())
}

pub async fn reconcile_turn_failed_on_provider_exit(
    state: &Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
    run_id: Option<RunId>,
    turn_id: TurnId,
    fallback_reason: &str,
) -> Result<()> {
    let store = state.store_for_session(session_id).await?;
    let turn = store.get_session_turn(session_id, turn_id).await?;
    let Some(turn) = turn else {
        return Ok(());
    };
    if matches!(
        turn.status,
        SessionTurnStatus::Completed | SessionTurnStatus::Failed | SessionTurnStatus::Interrupted
    ) {
        return Ok(());
    }

    let mut events = store
        .list_session_events_for_turn(session_id, turn_id, false)
        .await?;
    if resolve_terminal_state(&events).is_some() {
        return reconcile_turn_terminal_state(state, session_id, run_id, turn_id, fallback_reason)
            .await;
    }

    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        events = store
            .list_session_events_for_turn(session_id, turn_id, false)
            .await?;
        if resolve_terminal_state(&events).is_some() {
            return reconcile_turn_terminal_state(
                state,
                session_id,
                run_id,
                turn_id,
                fallback_reason,
            )
            .await;
        }
    }

    let failed_at = Utc::now();
    let message_id = turn.user_message_id.map(|id| id.0);
    let _ = emit_event(
        state,
        session_id,
        run_id,
        Some(turn_id),
        SessionEventType::Error,
        json!({
            "message_id": message_id,
            "error": "provider exited without emitting a terminal event",
            "reason": fallback_reason,
            "status": "failed",
        }),
    )
    .await;
    let _ = store
        .update_session_turn_status(
            session_id,
            turn_id,
            SessionTurnStatus::Failed,
            None,
            None,
            failed_at,
        )
        .await;
    let _ = emit_event(
        state,
        session_id,
        run_id,
        Some(turn_id),
        SessionEventType::TurnFinished,
        json!({
            "message_id": message_id,
            "status": "failed",
            "reason": fallback_reason,
        }),
    )
    .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use ctx_core::ids::{RunId, SessionEventId, SessionId, TurnId};
    use ctx_core::models::{SessionEvent, SessionEventType, SessionTurnStatus};

    use super::resolve_terminal_state;

    fn event(
        seq: i64,
        event_type: SessionEventType,
        payload_json: serde_json::Value,
    ) -> SessionEvent {
        SessionEvent {
            seq,
            id: SessionEventId::new(),
            session_id: SessionId::new(),
            run_id: Some(RunId::new()),
            turn_id: Some(TurnId::new()),
            event_type,
            payload_json,
            transient: false,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn turn_finished_status_overrides_completed_default() {
        let events = vec![event(
            4,
            SessionEventType::TurnFinished,
            json!({"status": "interrupted"}),
        )];

        let resolved = resolve_terminal_state(&events).expect("resolved state");
        assert_eq!(resolved.status, SessionTurnStatus::Interrupted);
        assert_eq!(resolved.end_seq, Some(4));
    }

    #[test]
    fn turn_finished_falls_back_to_prior_terminal_status() {
        let events = vec![
            event(3, SessionEventType::Error, json!({"status": "failed"})),
            event(4, SessionEventType::TurnFinished, json!({})),
        ];

        let resolved = resolve_terminal_state(&events).expect("resolved state");
        assert_eq!(resolved.status, SessionTurnStatus::Failed);
        assert_eq!(resolved.end_seq, Some(4));
    }

    #[test]
    fn turn_finished_keeps_done_metrics() {
        let events = vec![
            event(
                2,
                SessionEventType::Done,
                json!({"context_window": {"context_tokens_estimate": 42}}),
            ),
            event(
                3,
                SessionEventType::TurnFinished,
                json!({"status": "completed"}),
            ),
        ];

        let resolved = resolve_terminal_state(&events).expect("resolved state");
        assert_eq!(resolved.status, SessionTurnStatus::Completed);
        assert_eq!(
            resolved.metrics,
            Some(&json!({"context_tokens_estimate": 42}))
        );
    }
}
