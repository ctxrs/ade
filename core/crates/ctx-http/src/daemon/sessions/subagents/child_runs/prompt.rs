use std::sync::Arc;
use std::time::Instant;

use crate::daemon::scheduler::{QueuedMessage, SchedulerCommand};
use crate::daemon::AppState;
use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, Session, SessionEventType, SessionTurn,
    SessionTurnStatus,
};

use super::super::errors::{internal_api_error, ApiResult};

pub(in crate::daemon::sessions::subagents) struct PersistedSubagentPrompt {
    pub(in crate::daemon::sessions::subagents) run_id: RunId,
    pub(in crate::daemon::sessions::subagents) saved_message: Message,
    pub(in crate::daemon::sessions::subagents) last_event_seq: i64,
}

fn turn_status_has_input_backlog(status: &SessionTurnStatus) -> bool {
    matches!(
        status,
        SessionTurnStatus::Queued | SessionTurnStatus::Starting | SessionTurnStatus::Running
    )
}

pub(in crate::daemon::sessions::subagents) async fn persist_subagent_prompt(
    state: &Arc<AppState>,
    session: &Session,
    prompt: String,
) -> ApiResult<PersistedSubagentPrompt> {
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
    let has_backlog = state.sessions.is_running(session.id).await
        || !store
            .list_queued_messages_for_session(session.id)
            .await
            .map_err(internal_api_error)?
            .is_empty()
        || store
            .get_latest_turn_for_session(session.id)
            .await
            .map_err(internal_api_error)?
            .as_ref()
            .is_some_and(|turn| turn_status_has_input_backlog(&turn.status));
    let delivery = if has_backlog {
        MessageDelivery::Queued
    } else {
        MessageDelivery::Immediate
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
        delivery,
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
    let mut last_event_seq = start_seq;

    let turn = SessionTurn {
        turn_id,
        session_id: session.id,
        run_id: Some(run_id),
        user_message_id: Some(saved.id),
        status: match saved.delivery {
            MessageDelivery::Queued => SessionTurnStatus::Queued,
            MessageDelivery::Immediate => SessionTurnStatus::Starting,
        },
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
    if matches!(saved.delivery, MessageDelivery::Queued) {
        let queued = store
            .append_session_event(
                session.id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::InputQueued,
                serde_json::json!({"message_id": saved.id.0}),
            )
            .await
            .map_err(internal_api_error)?;
        state.publish_event(queued).await;

        let queue_position = store
            .list_queued_messages_for_session(session.id)
            .await
            .ok()
            .and_then(|messages| {
                messages
                    .iter()
                    .position(|message| message.id == saved.id)
                    .map(|idx| idx as i64)
            });

        let queue_added = store
            .append_session_event(
                session.id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::MessageQueueAdded,
                serde_json::json!({
                    "message_id": saved.id.0,
                    "queue_position": queue_position,
                }),
            )
            .await
            .map_err(internal_api_error)?;
        state.publish_event(queue_added).await;

        let turn_queued = store
            .append_session_event(
                session.id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::TurnQueued,
                serde_json::json!({
                    "message_id": saved.id.0,
                    "queue_position": queue_position,
                }),
            )
            .await
            .map_err(internal_api_error)?;
        last_event_seq = turn_queued.seq;
        state.publish_event(turn_queued).await;
    }

    Ok(PersistedSubagentPrompt {
        run_id,
        saved_message: saved,
        last_event_seq,
    })
}

pub(in crate::daemon::sessions::subagents) async fn dispatch_subagent_prompt(
    state: &Arc<AppState>,
    session: &Session,
    saved: &Message,
) {
    let tx = state.ensure_scheduler(session.clone()).await;
    let queued = QueuedMessage {
        message: saved.clone(),
        enqueued_at: Instant::now(),
        run_id: None,
    };
    let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;
}

pub(in crate::daemon::sessions::subagents) async fn enqueue_subagent_prompt(
    state: &Arc<AppState>,
    session: &Session,
    prompt: String,
) -> ApiResult<PersistedSubagentPrompt> {
    let persisted = persist_subagent_prompt(state, session, prompt).await?;
    dispatch_subagent_prompt(state, session, &persisted.saved_message).await;
    Ok(persisted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_backlog_includes_starting_turns() {
        assert!(turn_status_has_input_backlog(&SessionTurnStatus::Starting));
        assert!(turn_status_has_input_backlog(&SessionTurnStatus::Running));
        assert!(turn_status_has_input_backlog(&SessionTurnStatus::Queued));
        assert!(!turn_status_has_input_backlog(
            &SessionTurnStatus::Completed
        ));
    }
}
