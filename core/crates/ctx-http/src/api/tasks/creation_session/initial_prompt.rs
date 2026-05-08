use super::*;
use crate::api::sessions;

pub(super) struct InitialPromptSeed {
    pub(super) prompt: Option<String>,
    pub(super) message_id: Option<String>,
    pub(super) turn_id: Option<String>,
    pub(super) run_id_header: Option<String>,
}

pub(super) async fn seed_initial_prompt(
    state: &Arc<AppState>,
    store: &Store,
    session: &Session,
    seed: InitialPromptSeed,
) -> Result<(), StatusCode> {
    let Some(prompt) = seed.prompt else {
        return Ok(());
    };

    let (message_id, turn_id) = match (seed.message_id.as_deref(), seed.turn_id.as_deref()) {
        (Some(message_id), Some(turn_id)) => (
            MessageId(uuid::Uuid::parse_str(message_id).map_err(|_| StatusCode::BAD_REQUEST)?),
            TurnId(uuid::Uuid::parse_str(turn_id).map_err(|_| StatusCode::BAD_REQUEST)?),
        ),
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    let delivery = MessageDelivery::Immediate;
    let attachments = Vec::new();
    if let Some(existing) = store
        .get_message(message_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        let matches = existing.session_id == session.id
            && existing.turn_id == Some(turn_id)
            && matches!(existing.role, MessageRole::User)
            && existing.content == prompt
            && existing.attachments.is_empty()
            && matches!(existing.delivery, MessageDelivery::Immediate);
        if matches {
            sessions::ensure_session_turn_for_message(store, session.id, turn_id, &existing)
                .await?;
        } else {
            return Err(StatusCode::CONFLICT);
        }
    } else {
        let prompt_for_idempotency = prompt.clone();

        let run_id = RunId::new();
        let order_seq_state = state.sessions.get_order_seq_state(store, session.id).await;
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
            attachments: attachments.clone(),
            delivery,
            delivered_at: None,
            created_at: chrono::Utc::now(),
        };

        let saved = match store.insert_message(msg).await {
            Ok(saved) => saved,
            Err(err) if is_unique_constraint_violation(&err) => {
                let Some(existing) = store
                    .get_message(message_id)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                else {
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                };
                let matches = existing.session_id == session.id
                    && existing.turn_id == Some(turn_id)
                    && matches!(existing.role, MessageRole::User)
                    && existing.content == prompt_for_idempotency
                    && existing.attachments.is_empty()
                    && matches!(existing.delivery, MessageDelivery::Immediate);
                if matches {
                    existing
                } else {
                    return Err(StatusCode::CONFLICT);
                }
            }
            Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        };

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
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let start_seq = event.seq;

        let turn = SessionTurn {
            turn_id,
            session_id: session.id,
            run_id: Some(run_id),
            user_message_id: Some(saved.id),
            status: SessionTurnStatus::Starting,
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

        if let Err(err) = store.insert_session_turn(turn).await {
            if !is_unique_constraint_violation(&err) {
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
            let existing = store
                .get_session_turn_by_id(turn_id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            if let Some(existing) = existing {
                let matches =
                    existing.session_id == session.id && existing.user_message_id == Some(saved.id);
                if !matches {
                    return Err(StatusCode::CONFLICT);
                }
            } else {
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }

        state.publish_event(event).await;

        let prompt = saved.content.clone();
        let tx = state.ensure_scheduler(session.clone()).await;
        let queued = crate::daemon::scheduler::QueuedMessage {
            message: saved,
            enqueued_at: Instant::now(),
            run_id: seed.run_id_header.clone(),
        };
        let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

        let _ =
            schedule_session_title_generation(Arc::clone(state), session.clone(), prompt, false)
                .await;
    }
    Ok(())
}
