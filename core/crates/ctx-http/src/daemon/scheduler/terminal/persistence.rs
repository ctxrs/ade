use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use ctx_core::ids::{RunId, SessionId, TurnId};
use ctx_core::models::{RunStatus, SessionEvent, SessionEventType};

use crate::daemon::AppState;

use super::super::persistence::{
    is_transient_store_error, STORE_WRITE_RETRY_BASE_MS, STORE_WRITE_RETRY_LIMIT,
};

async fn publish_persisted_events(state: &Arc<AppState>, events: Vec<SessionEvent>) {
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

pub(super) async fn persist_terminal_events(
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
