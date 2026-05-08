use std::sync::{Arc, Weak};
use std::time::Instant;

use crate::daemon::AppState;
use ctx_core::ids::{RunId, SessionId, TurnId, WorktreeId};
#[cfg(test)]
use ctx_core::models::SessionEvent;
use ctx_core::models::{SessionEventType, SessionTurn, SessionTurnStatus, SubagentInvocationChild};
#[cfg(test)]
use ctx_core::session_projection::turn_status_from_finished_payload;
use ctx_observability::logs;

use super::errors::{internal_api_error, store_for_session, ApiResult};

mod prompt;

pub(super) use prompt::{
    dispatch_subagent_prompt, enqueue_subagent_prompt, persist_subagent_prompt,
    PersistedSubagentPrompt,
};

pub(super) fn subagent_status_from_turn_status(status: SessionTurnStatus) -> &'static str {
    match status {
        SessionTurnStatus::Completed => "completed",
        SessionTurnStatus::Interrupted => "interrupted",
        SessionTurnStatus::Failed => "failed",
        SessionTurnStatus::Starting | SessionTurnStatus::Running | SessionTurnStatus::Queued => {
            "running"
        }
    }
}

fn subagent_terminal_status_from_turn_status(status: SessionTurnStatus) -> Option<&'static str> {
    match status {
        SessionTurnStatus::Completed => Some("completed"),
        SessionTurnStatus::Interrupted => Some("interrupted"),
        SessionTurnStatus::Failed => Some("failed"),
        SessionTurnStatus::Starting | SessionTurnStatus::Running | SessionTurnStatus::Queued => {
            None
        }
    }
}

#[cfg(test)]
fn subagent_terminal_status_from_event(event: &SessionEvent) -> Option<&'static str> {
    match event.event_type {
        SessionEventType::Done => Some("completed"),
        SessionEventType::TurnInterrupted => Some("interrupted"),
        SessionEventType::TurnFinished => turn_status_from_finished_payload(&event.payload_json)
            .and_then(subagent_terminal_status_from_turn_status),
        _ => None,
    }
}

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

pub(super) async fn wait_for_run_terminal_turn(
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

pub(super) async fn wait_for_run_assistant_message(
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

pub(super) async fn emit_subagent_invocation_notice(
    state: &Arc<AppState>,
    parent_session_id: SessionId,
    parent_turn_id: Option<TurnId>,
    payload: serde_json::Value,
) -> ApiResult<()> {
    let store = store_for_session(state.as_ref(), parent_session_id).await?;
    let event = store
        .append_session_event(
            parent_session_id,
            None,
            parent_turn_id,
            SessionEventType::Notice,
            payload,
        )
        .await
        .map_err(internal_api_error)?;
    state.publish_event(event).await;
    Ok(())
}

pub(super) async fn run_subagent_child(
    state_weak: &Weak<AppState>,
    child: SubagentInvocationChild,
    _parent_worktree_id: WorktreeId,
) -> Result<(), String> {
    let run_id = child
        .run_id
        .ok_or_else(|| "subagent run_id missing".to_string())?;
    let Some(state) = state_weak.upgrade() else {
        return Ok(());
    };
    let store = state
        .store_for_session(child.child_session_id)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?;
    let status = match wait_for_run_terminal_turn(
        state_weak,
        &store,
        child.child_session_id,
        run_id,
    )
    .await
    {
        Ok(Some(turn)) => subagent_status_from_turn_status(turn.status).to_string(),
        Ok(None) => return Ok(()),
        Err(_) => "unknown".to_string(),
    };

    let child_updated_at = chrono::Utc::now();
    let mut updated_child = child.clone();
    updated_child.status = status.clone();
    updated_child.updated_at = child_updated_at;
    store
        .upsert_subagent_invocation_child(updated_child)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?;
    Ok(())
}

pub(super) async fn finalize_subagent_invocation(
    state: &Arc<AppState>,
    invocation_id: &str,
    tool_call_id: &str,
    parent_session_id: SessionId,
    parent_turn_id: Option<TurnId>,
) -> Result<(), String> {
    let store = state
        .store_for_session(parent_session_id)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?;
    let Some(invocation) = store
        .get_subagent_invocation(invocation_id)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?
    else {
        return Ok(());
    };

    if invocation.children.is_empty() {
        return Ok(());
    }
    if invocation
        .children
        .iter()
        .any(|child| child.status == "running")
    {
        return Ok(());
    }

    let final_status = if invocation
        .children
        .iter()
        .all(|child| child.status == "completed")
    {
        "completed"
    } else {
        "failed"
    };
    if invocation.status == final_status {
        return Ok(());
    }

    let updated_at = chrono::Utc::now();
    store
        .update_subagent_invocation_status(invocation_id, final_status, updated_at)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?;
    let child_session_ids = invocation
        .children
        .iter()
        .map(|child| child.child_session_id.0.to_string())
        .collect::<Vec<_>>();
    let child_statuses = invocation
        .children
        .iter()
        .map(|child| {
            serde_json::json!({
                "session_id": child.child_session_id.0.to_string(),
                "status": child.status,
            })
        })
        .collect::<Vec<_>>();
    emit_subagent_invocation_notice(
        state,
        parent_session_id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_updated",
            "invocation_id": invocation_id,
            "tool_call_id": tool_call_id,
            "status": final_status,
            "child_session_ids": child_session_ids,
            "child_statuses": child_statuses,
        }),
    )
    .await
    .map_err(|error| error.message().to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    use serde_json::json;

    use ctx_core::ids::SessionEventId;

    fn terminal_event(status: &str) -> SessionEvent {
        SessionEvent {
            seq: 1,
            id: SessionEventId::new(),
            session_id: SessionId::new(),
            run_id: Some(RunId::new()),
            turn_id: Some(TurnId::new()),
            event_type: SessionEventType::TurnFinished,
            payload_json: json!({ "status": status }),
            transient: false,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn subagent_terminal_status_uses_turn_finished_failed_payload() {
        assert_eq!(
            subagent_terminal_status_from_event(&terminal_event("failed")),
            Some("failed")
        );
    }

    #[test]
    fn subagent_terminal_status_uses_turn_finished_interrupted_payload() {
        assert_eq!(
            subagent_terminal_status_from_event(&terminal_event("interrupted")),
            Some("interrupted")
        );
    }
}
