use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use ctx_core::ids::{RunId, TurnId};
use ctx_core::models::{SessionEventType, SessionTurnStatus};
use ctx_core::session_projection::resolve_turn_terminal_state;

use crate::daemon::scheduler::persistence::{emit_event, flush_session_events};
use crate::daemon::AppState;

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
        SessionTurnStatus::Queued
            | SessionTurnStatus::Completed
            | SessionTurnStatus::Failed
            | SessionTurnStatus::Interrupted
    ) {
        state.set_running(session_id, false).await;
        return Ok(());
    }

    let events = store
        .list_session_events_for_turn(session_id, turn_id, false)
        .await?;
    if resolve_turn_terminal_state(&events).is_some() {
        let _ = store
            .repair_session_turn_projection_from_events(session_id, turn_id)
            .await;
        state.set_running(session_id, false).await;
        return Ok(());
    }

    let persisted = store
        .persist_turn_terminal_events(
            session_id,
            run_id,
            turn_id,
            vec![
                (
                    SessionEventType::TurnInterrupted,
                    json!({
                        "reason": fallback_reason,
                        "provider_cancelled": false,
                        "status": "interrupted",
                    }),
                ),
                (
                    SessionEventType::TurnFinished,
                    json!({
                        "message_id": turn.user_message_id.map(|id| id.0),
                        "status": "interrupted",
                        "reason": fallback_reason,
                        "provider_cancelled": false,
                    }),
                ),
            ],
        )
        .await?;
    for event in persisted {
        state.publish_event(event).await;
    }
    state.set_running(session_id, false).await;
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
        SessionTurnStatus::Queued
            | SessionTurnStatus::Completed
            | SessionTurnStatus::Failed
            | SessionTurnStatus::Interrupted
    ) {
        return Ok(());
    }

    let mut events = store
        .list_session_events_for_turn(session_id, turn_id, false)
        .await?;
    if resolve_turn_terminal_state(&events).is_some() {
        return reconcile_turn_terminal_state(state, session_id, run_id, turn_id, fallback_reason)
            .await;
    }

    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        events = store
            .list_session_events_for_turn(session_id, turn_id, false)
            .await?;
        if resolve_turn_terminal_state(&events).is_some() {
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
    flush_session_events(&store, session_id, "reconcile_turn_failed_on_provider_exit").await;
    let _ = store
        .repair_session_turn_projection_from_events(session_id, turn_id)
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
    let _ = store
        .repair_session_turn_projection_from_events(session_id, turn_id)
        .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use ctx_core::ids::{RunId, SessionEventId, SessionId, TurnId};
    use ctx_core::models::{SessionEvent, SessionEventType, SessionTurnStatus};

    use ctx_core::session_projection::resolve_turn_terminal_state;

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

        let resolved = resolve_turn_terminal_state(&events).expect("resolved state");
        assert_eq!(resolved.status, SessionTurnStatus::Interrupted);
        assert_eq!(resolved.end_seq, Some(4));
    }

    #[test]
    fn turn_finished_without_status_does_not_resolve_terminal_state() {
        let events = vec![
            event(3, SessionEventType::Error, json!({"status": "failed"})),
            event(4, SessionEventType::TurnFinished, json!({})),
        ];

        assert!(resolve_turn_terminal_state(&events).is_none());
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

        let resolved = resolve_turn_terminal_state(&events).expect("resolved state");
        assert_eq!(resolved.status, SessionTurnStatus::Completed);
        assert_eq!(
            resolved.metrics,
            Some(json!({"context_tokens_estimate": 42}))
        );
    }
}
