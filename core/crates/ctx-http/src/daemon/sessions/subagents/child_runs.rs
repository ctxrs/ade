use std::sync::Arc;
use std::time::Instant;

use crate::api::sessions::{build_subagent_result, context_window_for_run, AgentInitResult};
use crate::daemon::AppState;
use crate::logs;
use crate::scheduler::{QueuedMessage, SchedulerCommand};
use ctx_core::ids::{MessageId, RunId, SessionId, TurnId, WorktreeId};
#[cfg(test)]
use ctx_core::models::SessionEvent;
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, Session, SessionEventType, SessionTurn,
    SessionTurnStatus, SubagentInvocationChild,
};
#[cfg(test)]
use ctx_core::session_projection::turn_status_from_finished_payload;

use super::errors::{internal_api_error, store_for_session, ApiResult};

pub(super) fn subagent_status_from_turn_status(status: SessionTurnStatus) -> &'static str {
    match status {
        SessionTurnStatus::Completed => "completed",
        SessionTurnStatus::Interrupted => "interrupted",
        SessionTurnStatus::Failed => "failed",
        SessionTurnStatus::Running | SessionTurnStatus::Queued => "running",
    }
}

fn subagent_terminal_status_from_turn_status(status: SessionTurnStatus) -> Option<&'static str> {
    match status {
        SessionTurnStatus::Completed => Some("completed"),
        SessionTurnStatus::Interrupted => Some("interrupted"),
        SessionTurnStatus::Failed => Some("failed"),
        SessionTurnStatus::Running | SessionTurnStatus::Queued => None,
    }
}

#[cfg(test)]
fn subagent_terminal_status_from_event(event: &SessionEvent) -> Option<&'static str> {
    match event.event_type {
        SessionEventType::Done => Some("completed"),
        SessionEventType::Error => Some("failed"),
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
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: RunId,
) -> Result<SessionTurn, String> {
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?;
    let mut rx = state.subscribe_session_event_head(session_id).await;
    if let Some(turn) = latest_terminal_turn_for_run(&store, session_id, run_id).await? {
        return Ok(turn);
    }

    loop {
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    rx = state.subscribe_session_event_head(session_id).await;
                }
                if let Some(turn) = latest_terminal_turn_for_run(&store, session_id, run_id).await?
                {
                    return Ok(turn);
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {
                if let Some(turn) = latest_terminal_turn_for_run(&store, session_id, run_id).await?
                {
                    return Ok(turn);
                }
            }
        }
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
    state: &Arc<AppState>,
    child: SubagentInvocationChild,
    parent_worktree_id: WorktreeId,
) -> Result<AgentInitResult, String> {
    let run_id = child
        .run_id
        .ok_or_else(|| "subagent run_id missing".to_string())?;
    let store = state
        .store_for_session(child.child_session_id)
        .await
        .map_err(|error| logs::redact_sensitive(&error.to_string()))?;
    let status = match wait_for_run_terminal_turn(state, child.child_session_id, run_id).await {
        Ok(turn) => subagent_status_from_turn_status(turn.status).to_string(),
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

    let content = store
        .get_last_assistant_message_for_run(child.child_session_id, run_id)
        .await
        .ok()
        .flatten()
        .map(|message| message.content);

    let context_window = context_window_for_run(state, child.child_session_id, run_id).await;
    build_subagent_result(
        state,
        parent_worktree_id,
        &child,
        status,
        content,
        context_window,
    )
    .await
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
    .map_err(|(_, error)| error.0.error)?;

    Ok(())
}

pub(super) async fn enqueue_subagent_prompt(
    state: &Arc<AppState>,
    session: &Session,
    prompt: String,
) -> ApiResult<(RunId, Message)> {
    let store = state
        .store_for_session(session.id)
        .await
        .map_err(internal_api_error)?;
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let message_id = MessageId::new();
    let order_seq_state = state.sessions.get_order_seq_state(&store, session.id).await;
    let order_seq = {
        let mut order_seq_state = order_seq_state.lock().await;
        order_seq_state.get_or_assign(format!("message:{}", message_id.0), None)
    };
    let msg = Message {
        id: message_id,
        session_id: session.id,
        task_id: session.task_id,
        run_id: Some(run_id),
        turn_id: Some(turn_id),
        turn_sequence: Some(0),
        order_seq: Some(order_seq),
        role: MessageRole::User,
        content: prompt,
        attachments: vec![],
        delivery: MessageDelivery::Immediate,
        delivered_at: None,
        created_at: chrono::Utc::now(),
    };
    let saved = store
        .insert_message(msg)
        .await
        .map_err(internal_api_error)?;
    let event = store
        .append_session_event(
            session.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::UserMessage,
            serde_json::json!({
                "message_id": saved.id.0,
                "content": saved.content.clone(),
                "delivery": saved.delivery.clone(),
                "attachments": saved.attachments,
                "order_seq": order_seq,
            }),
        )
        .await
        .map_err(internal_api_error)?;
    let start_seq = event.seq;

    let turn = SessionTurn {
        turn_id,
        session_id: session.id,
        run_id: Some(run_id),
        user_message_id: Some(saved.id),
        status: SessionTurnStatus::Running,
        start_seq: Some(start_seq),
        end_seq: None,
        started_at: saved.created_at,
        updated_at: saved.created_at,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    };
    let _ = store.insert_session_turn(turn).await;
    state.publish_event(event).await;

    let tx = state.ensure_scheduler(session.clone()).await;
    let queued = QueuedMessage {
        message: saved.clone(),
        enqueued_at: Instant::now(),
        run_id: None,
    };
    let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

    Ok((run_id, saved))
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
