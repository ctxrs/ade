use std::sync::Arc;

use anyhow::Result;

use crate::daemon::AppState;
use ctx_core::ids::{RunId, SessionId, TurnId};
use ctx_core::models::SessionTurnStatus;
use ctx_core::session_projection::resolve_turn_terminal_state;

mod events;

use events::fallback_interrupted_turn_events;

pub async fn reconcile_turn_terminal_state(
    state: &Arc<AppState>,
    session_id: SessionId,
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
            fallback_interrupted_turn_events(turn.user_message_id, fallback_reason),
        )
        .await?;
    for event in persisted {
        state.publish_event(event).await;
    }
    state.set_running(session_id, false).await;
    Ok(())
}
