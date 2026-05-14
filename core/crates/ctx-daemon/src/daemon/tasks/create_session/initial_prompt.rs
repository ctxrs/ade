use super::*;

#[path = "initial_prompt/records.rs"]
mod records;

use self::records::{
    existing_initial_prompt_message_matches, initial_prompt_turn,
    initial_prompt_user_event_payload, new_initial_prompt_message, parse_initial_prompt_ids,
};

pub(super) struct InitialPromptSeed {
    pub(super) prompt: Option<String>,
    pub(super) message_id: Option<String>,
    pub(super) turn_id: Option<String>,
    pub(super) run_id_header: Option<String>,
}

pub(super) async fn seed_initial_prompt(
    handles: &TaskSessionHandles,
    store: &Store,
    session: &Session,
    seed: InitialPromptSeed,
) -> Result<(), TaskSessionCreateError> {
    let Some(prompt) = seed.prompt else {
        return Ok(());
    };

    let ids = parse_initial_prompt_ids(seed.message_id.as_deref(), seed.turn_id.as_deref())?;
    let message_id = ids.message_id;
    let turn_id = ids.turn_id;

    let delivery = MessageDelivery::Immediate;
    if let Some(existing) = store
        .get_message(message_id)
        .await
        .map_err(TaskSessionCreateError::Internal)?
    {
        if existing_initial_prompt_message_matches(&existing, session, turn_id, &prompt) {
            ensure_session_turn_for_initial_prompt(store, session.id, turn_id, &existing).await?;
        } else {
            return Err(TaskSessionCreateError::Conflict);
        }
    } else {
        let prompt_for_idempotency = prompt.clone();

        let run_id = RunId::new();
        let order_seq_state = handles
            .sessions
            .session_order_seq_state(store, session.id)
            .await;
        let order_seq = {
            let mut order_seq_state = order_seq_state.lock().await;
            order_seq_state.get_or_assign(format!("message:{}", message_id.0), None)
        };
        let msg = new_initial_prompt_message(session, ids, run_id, prompt, order_seq, delivery);

        let saved = match store.insert_message(msg).await {
            Ok(saved) => saved,
            Err(err) if is_unique_constraint_violation(&err) => {
                let Some(existing) = store
                    .get_message(message_id)
                    .await
                    .map_err(TaskSessionCreateError::Internal)?
                else {
                    return Err(TaskSessionCreateError::Internal(anyhow::anyhow!(
                        "message insert conflicted but message row is missing"
                    )));
                };
                if existing_initial_prompt_message_matches(
                    &existing,
                    session,
                    turn_id,
                    &prompt_for_idempotency,
                ) {
                    existing
                } else {
                    return Err(TaskSessionCreateError::Conflict);
                }
            }
            Err(error) => return Err(TaskSessionCreateError::Internal(error)),
        };

        let event = store
            .append_session_event(
                session.id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::UserMessage,
                initial_prompt_user_event_payload(&saved, order_seq),
            )
            .await
            .map_err(TaskSessionCreateError::Internal)?;
        let start_seq = event.seq;

        let turn = initial_prompt_turn(session, ids, run_id, &saved, start_seq);

        if let Err(err) = store.insert_session_turn(turn).await {
            if !is_unique_constraint_violation(&err) {
                return Err(TaskSessionCreateError::Internal(err));
            }
            let existing = store
                .get_session_turn_by_id(turn_id)
                .await
                .map_err(TaskSessionCreateError::Internal)?;
            if let Some(existing) = existing {
                let matches =
                    existing.session_id == session.id && existing.user_message_id == Some(saved.id);
                if !matches {
                    return Err(TaskSessionCreateError::Conflict);
                }
            } else {
                return Err(TaskSessionCreateError::Internal(anyhow::anyhow!(
                    "session turn insert conflicted but turn row is missing"
                )));
            }
        }

        handles.sessions.publish_event(event).await;

        let prompt = saved.content.clone();
        let tx = handles.sessions.ensure_scheduler(session.clone()).await;
        let queued = crate::daemon::scheduler::QueuedMessage {
            message: saved,
            enqueued_at: Instant::now(),
            run_id: seed.run_id_header.clone(),
        };
        let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

        let _ = handles
            .sessions
            .schedule_session_title_generation(session.clone(), prompt, false)
            .await;
    }
    Ok(())
}

async fn ensure_session_turn_for_initial_prompt(
    store: &Store,
    session_id: SessionId,
    turn_id: TurnId,
    message: &Message,
) -> Result<(), TaskSessionCreateError> {
    let existing_turn = store
        .get_session_turn_by_id(turn_id)
        .await
        .map_err(TaskSessionCreateError::Internal)?;
    if let Some(existing) = existing_turn {
        let matches =
            existing.session_id == session_id && existing.user_message_id == Some(message.id);
        if !matches {
            return Err(TaskSessionCreateError::Conflict);
        }
        return Ok(());
    }

    let turn =
        ctx_session_service::message_delivery::build_user_message_turn(message, turn_id, None);
    if let Err(err) = store.insert_session_turn(turn).await {
        if !is_unique_constraint_violation(&err) {
            return Err(TaskSessionCreateError::Internal(err));
        }
        let existing = store
            .get_session_turn_by_id(turn_id)
            .await
            .map_err(TaskSessionCreateError::Internal)?;
        if let Some(existing) = existing {
            let matches =
                existing.session_id == session_id && existing.user_message_id == Some(message.id);
            if !matches {
                return Err(TaskSessionCreateError::Conflict);
            }
        } else {
            return Err(TaskSessionCreateError::Internal(anyhow::anyhow!(
                "session turn insert conflicted but turn row is missing"
            )));
        }
    }
    Ok(())
}
