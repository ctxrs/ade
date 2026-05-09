use std::sync::{Arc, Weak};
use std::time::Instant;

use ctx_core::ids::{RunId, SessionId};
use ctx_core::models::SessionTurn;
use ctx_observability::logs;

use crate::daemon::AppState;

use super::status::subagent_terminal_status_from_turn_status;

async fn latest_terminal_turn_for_run(
    store: &ctx_store::Store,
    session_id: SessionId,
    run_id: RunId,
) -> Result<Option<SessionTurn>, String> {
    let turn = store
        .get_latest_turn_for_run(session_id, run_id)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?;
    Ok(turn.and_then(|turn| {
        subagent_terminal_status_from_turn_status(turn.status.clone()).map(|_| turn)
    }))
}

pub(in crate::daemon::sessions::subagents) async fn wait_for_run_terminal_turn(
    state_weak: &Weak<AppState>,
    store: &ctx_store::Store,
    session_id: SessionId,
    run_id: RunId,
) -> Result<Option<SessionTurn>, String> {
    if let Some(turn) = latest_terminal_turn_for_run(store, session_id, run_id).await? {
        return Ok(Some(turn));
    }
    let Some(state) = state_weak.upgrade() else {
        return Ok(None);
    };
    let mut rx = state
        .sessions
        .subscribe_session_event_head(session_id)
        .await;
    drop(state);

    loop {
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    let Some(state) = state_weak.upgrade() else {
                        return Ok(None);
                    };
                    rx = state.sessions.subscribe_session_event_head(session_id).await;
                }
                if let Some(turn) = latest_terminal_turn_for_run(store, session_id, run_id).await?
                {
                    return Ok(Some(turn));
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {
                if let Some(turn) = latest_terminal_turn_for_run(store, session_id, run_id).await?
                {
                    return Ok(Some(turn));
                }
            }
        }
    }
}

pub(in crate::daemon::sessions::subagents) async fn wait_for_run_assistant_message(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: RunId,
) -> Result<Option<String>, String> {
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?;
    let deadline = Instant::now() + std::time::Duration::from_secs(2);

    loop {
        if let Some(message) = store
            .get_last_assistant_message_for_run(session_id, run_id)
            .await
            .map_err(|error| logs::redact_sensitive(&error.to_string()))?
        {
            return Ok(Some(message.content));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
