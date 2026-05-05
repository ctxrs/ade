use super::attachments::{attachments_match, normalize_message_attachments};
use super::turns::{delivery_matches, ensure_session_turn_for_message};
use super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct PostMessageReq {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    turn_id: Option<String>,
    content: String,
    delivery: Option<MessageDelivery>,
    #[serde(default)]
    attachments: Vec<MessageAttachment>,
}

const QUEUED_MESSAGES_ENABLED_ENV: &str = "CTX_QUEUED_MESSAGES_ENABLED";

fn env_bool(key: &str) -> Option<bool> {
    std::env::var(key)
        .ok()
        .and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
}

fn queued_messages_enabled() -> bool {
    env_bool(QUEUED_MESSAGES_ENABLED_ENV).unwrap_or(false)
}

fn resolve_message_delivery(
    requested_delivery: Option<MessageDelivery>,
    session_running: bool,
    queued_enabled: bool,
) -> Result<MessageDelivery, ApiErr> {
    match requested_delivery {
        Some(MessageDelivery::Queued) if queued_enabled => Ok(MessageDelivery::Queued),
        Some(MessageDelivery::Queued) => Err(api_error(
            StatusCode::BAD_REQUEST,
            "Queued messages are disabled.",
        )),
        None if session_running && queued_enabled => Ok(MessageDelivery::Queued),
        Some(MessageDelivery::Immediate) | None if session_running => Err(api_error(
            StatusCode::CONFLICT,
            "A turn is already running. Stop it or wait for it to finish.",
        )),
        Some(MessageDelivery::Immediate) | None => Ok(MessageDelivery::Immediate),
    }
}

pub(crate) async fn delete_session_message(
    State(state): State<Arc<AppState>>,
    Path((session_id, id)): Path<(String, String)>,
) -> Result<StatusCode, StatusCode> {
    let session_id =
        SessionId(uuid::Uuid::parse_str(&session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let msg_id = MessageId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = store_for_existing_session_status_for_write(&state, session_id).await?;
    let msg = store
        .get_message(msg_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if msg.session_id != session_id {
        return Err(StatusCode::NOT_FOUND);
    }

    if !matches!(msg.delivery, MessageDelivery::Queued) || msg.delivered_at.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    store
        .delete_message(msg_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(turn_id) = msg.turn_id {
        let _ = store.delete_session_turn(msg.session_id, turn_id).await;
    }
    let removed = store
        .append_session_event(
            msg.session_id,
            msg.run_id,
            msg.turn_id,
            SessionEventType::MessageQueueRemoved,
            serde_json::json!({
                "message_id": msg.id.0,
                "reason": "user_delete",
            }),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.publish_event(removed).await;

    if let Some(tx) = state.scheduler_sender(msg.session_id).await {
        let _ = tx.send(SchedulerCommand::RemoveQueued(msg_id)).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

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
    state.remember_session_meta(&session).await;
    if let Some(drain) = state.update_drain_snapshot().await {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "Daemon update is in progress; retry after the daemon restarts. ({})",
                drain.reason
            ),
        ));
    }

    let requested_delivery = req.delivery.clone();
    let delivery = resolve_message_delivery(
        requested_delivery.clone(),
        state.is_running(session_id).await,
        queued_messages_enabled(),
    )?;

    let (message_id, turn_id, client_ids) = match (
        req.id.as_deref().map(str::trim).filter(|v| !v.is_empty()),
        req.turn_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty()),
    ) {
        (Some(message_id), Some(turn_id)) => (
            MessageId(
                uuid::Uuid::parse_str(message_id)
                    .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid message id."))?,
            ),
            TurnId(
                uuid::Uuid::parse_str(turn_id)
                    .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid turn id."))?,
            ),
            true,
        ),
        (None, None) => (MessageId::new(), TurnId::new(), false),
        _ => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "Message id and turn id must either both be provided or both be omitted.",
            ))
        }
    };

    let attachments = normalize_message_attachments(&state, req.attachments).await?;
    let content = req.content;

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

    let turn = SessionTurn {
        turn_id,
        session_id,
        run_id: Some(run_id),
        user_message_id: Some(saved.id),
        status: match &saved.delivery {
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
    let queued = crate::scheduler::QueuedMessage {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_resolution_does_not_queue_when_queue_feature_is_disabled() {
        let err = resolve_message_delivery(None, true, false)
            .expect_err("running sessions should reject implicit queueing while disabled");
        let (status, Json(body)) = err;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            body.error,
            "A turn is already running. Stop it or wait for it to finish."
        );

        let err = resolve_message_delivery(Some(MessageDelivery::Queued), false, false)
            .expect_err("explicit queued delivery should be gated server-side");
        let (status, Json(body)) = err;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.error, "Queued messages are disabled.");
    }

    #[test]
    fn delivery_resolution_queues_only_when_feature_enabled() {
        assert!(matches!(
            resolve_message_delivery(None, true, true).expect("implicit queueing enabled"),
            MessageDelivery::Queued
        ));
        assert!(matches!(
            resolve_message_delivery(Some(MessageDelivery::Queued), true, true)
                .expect("explicit queueing enabled"),
            MessageDelivery::Queued
        ));
        assert!(matches!(
            resolve_message_delivery(None, false, false).expect("idle sessions send immediately"),
            MessageDelivery::Immediate
        ));

        let err = resolve_message_delivery(Some(MessageDelivery::Immediate), true, true)
            .expect_err("explicit immediate delivery must not be silently queued");
        let (status, _) = err;
        assert_eq!(status, StatusCode::CONFLICT);
    }
}
