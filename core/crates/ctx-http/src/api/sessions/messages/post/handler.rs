use super::super::attachments::{attachments_match, normalize_message_attachments};
use super::super::turns::ensure_session_turn_for_message;
use super::super::*;
use super::delivery::{queued_messages_enabled, resolve_message_delivery};
use super::request::PostMessageReq;
use ctx_session_service::message_delivery::{
    build_user_message_turn, delivery_matches, resolve_message_client_ids,
    MessageClientIdResolutionError,
};

pub(crate) async fn post_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<PostMessageReq>,
) -> Result<Json<Message>, ApiErr> {
    let session_id = SessionId(
        uuid::Uuid::parse_str(&id)
            .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid session id."))?,
    );
    let store = store_for_existing_session_status_for_write(&state, session_id)
        .await
        .map_err(session_store_api_error)?;
    let run_id_header = headers
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load session."))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "Session not found."))?;
    state.sessions.remember_session_meta(&session).await;
    if let Some(drain) = state.core.update_drain.snapshot().await {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "Daemon update is in progress; retry after the daemon restarts. ({})",
                drain.reason
            ),
        ));
    }

    let request_message_id = req
        .id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            uuid::Uuid::parse_str(value)
                .map(MessageId)
                .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid message id."))
        })
        .transpose()?;
    let request_turn_id = req
        .turn_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            uuid::Uuid::parse_str(value)
                .map(TurnId)
                .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid turn id."))
        })
        .transpose()?;
    let client_ids = resolve_message_client_ids(request_message_id, request_turn_id).map_err(
        |error| match error {
            MessageClientIdResolutionError::PartialClientIds => {
                api_error(StatusCode::BAD_REQUEST, error.message())
            }
        },
    )?;
    let message_id = client_ids.message_id;
    let turn_id = client_ids.turn_id;
    let client_ids = client_ids.client_supplied;

    let attachments = normalize_message_attachments(&state, req.attachments).await?;
    let content = req.content;
    let requested_delivery = req.delivery.clone();

    if client_ids {
        if let Some(existing) = store
            .get_message(message_id)
            .await
            .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load message."))?
        {
            let matches = existing.session_id == session_id
                && existing.turn_id == Some(turn_id)
                && matches!(existing.role, MessageRole::User)
                && existing.content == content
                && match requested_delivery.as_ref() {
                    Some(requested_delivery) => {
                        delivery_matches(&existing.delivery, requested_delivery)
                    }
                    None => true,
                }
                && attachments_match(&state, &existing.attachments, &attachments).await?;
            if matches {
                ensure_session_turn_for_message(&store, session_id, turn_id, &existing)
                    .await
                    .map_err(|status| match status {
                        StatusCode::CONFLICT => api_error(
                            StatusCode::CONFLICT,
                            "Turn id already belongs to another message.",
                        ),
                        _ => api_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Failed to ensure message turn.",
                        ),
                    })?;
                return Ok(Json(existing));
            }
            return Err(api_error(
                StatusCode::CONFLICT,
                "A different message already exists for that client id.",
            ));
        }
    }

    let delivery = resolve_message_delivery(
        requested_delivery.clone(),
        state.sessions.is_running(session_id).await,
        queued_messages_enabled(),
    )?;

    let run_id = RunId::new();
    let order_seq_state = state.sessions.get_order_seq_state(&store, session_id).await;
    let order_seq = {
        let mut order_seq_state = order_seq_state.lock().await;
        order_seq_state.get_or_assign(format!("message:{}", message_id.0), None)
    };
    let idempotency_payload = client_ids.then(|| {
        (
            content.clone(),
            attachments.clone(),
            requested_delivery.clone(),
        )
    });
    let msg = Message {
        id: message_id,
        session_id,
        task_id: session.task_id,
        run_id: Some(run_id),
        turn_id: Some(turn_id),
        turn_sequence: Some(0),
        order_seq: Some(order_seq),
        role: MessageRole::User,
        content,
        attachments,
        delivery,
        delivered_at: None,
        created_at: chrono::Utc::now(),
    };
    let saved = match store.insert_message(msg).await {
        Ok(saved) => saved,
        Err(err) if idempotency_payload.is_some() && is_unique_constraint_violation(&err) => {
            let Some((content, attachments, requested_delivery)) = idempotency_payload else {
                return Err(api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Message idempotency state was unexpectedly missing.",
                ));
            };
            let Some(existing) = store.get_message(message_id).await.map_err(|_| {
                api_error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to load message.")
            })?
            else {
                return Err(api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Message already existed but could not be loaded.",
                ));
            };

            let matches = existing.session_id == session_id
                && existing.turn_id == Some(turn_id)
                && matches!(existing.role, MessageRole::User)
                && existing.content == content
                && match requested_delivery.as_ref() {
                    Some(requested_delivery) => {
                        delivery_matches(&existing.delivery, requested_delivery)
                    }
                    None => true,
                }
                && attachments_match(&state, &existing.attachments, &attachments).await?;
            if matches {
                existing
            } else {
                return Err(api_error(
                    StatusCode::CONFLICT,
                    "A different message already exists for that client id.",
                ));
            }
        }
        Err(_) => {
            return Err(api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to save message.",
            ))
        }
    };
    let event = store
        .append_session_event(
            session_id,
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
        .map_err(|_| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to append session event.",
            )
        })?;
    let start_seq = event.seq;

    let turn = build_user_message_turn(&saved, turn_id, Some(start_seq));
    let existing_turn = store.get_session_turn_by_id(turn_id).await.map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to inspect session turn.",
        )
    })?;
    if let Some(existing) = existing_turn {
        let matches =
            existing.session_id == session_id && existing.user_message_id == Some(saved.id);
        if !matches {
            return Err(api_error(
                StatusCode::CONFLICT,
                "Turn id already belongs to another message.",
            ));
        }
    } else if let Err(err) = store.insert_session_turn(turn).await {
        if !is_unique_constraint_violation(&err) {
            return Err(api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to create session turn.",
            ));
        }
        let existing = store.get_session_turn_by_id(turn_id).await.map_err(|_| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to inspect session turn.",
            )
        })?;
        if let Some(existing) = existing {
            let matches =
                existing.session_id == session_id && existing.user_message_id == Some(saved.id);
            if !matches {
                return Err(api_error(
                    StatusCode::CONFLICT,
                    "Turn id already belongs to another message.",
                ));
            }
        } else {
            return Err(api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Session turn insert succeeded but could not be reloaded.",
            ));
        }
    }
    state.publish_event(event).await;

    if matches!(saved.delivery, MessageDelivery::Queued) {
        let queued = store
            .append_session_event(
                session_id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::InputQueued,
                serde_json::json!({"message_id": saved.id.0}),
            )
            .await
            .map_err(|_| {
                api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to append queued-input event.",
                )
            })?;
        state.publish_event(queued).await;

        let queue_position = store
            .list_queued_messages_for_session(session_id)
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
                session_id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::MessageQueueAdded,
                serde_json::json!({
                    "message_id": saved.id.0,
                    "queue_position": queue_position,
                }),
            )
            .await
            .map_err(|_| {
                api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to append queue event.",
                )
            })?;
        state.publish_event(queue_added).await;

        let turn_queued = store
            .append_session_event(
                session_id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::TurnQueued,
                serde_json::json!({
                    "message_id": saved.id.0,
                    "queue_position": queue_position,
                }),
            )
            .await
            .map_err(|_| {
                api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to append queued turn event.",
                )
            })?;
        state.publish_event(turn_queued).await;
    }

    let tx = state.ensure_scheduler(session.clone()).await;
    let queued = crate::daemon::scheduler::QueuedMessage {
        message: saved.clone(),
        enqueued_at: Instant::now(),
        run_id: run_id_header.clone(),
    };
    let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

    if let Ok(count) = store.count_user_messages_for_session(session_id).await {
        if count == 1 {
            let prompt = saved.content.clone();
            let _ =
                schedule_session_title_generation(state.clone(), session.clone(), prompt, false)
                    .await;
        }
    }

    Ok(Json(saved))
}
