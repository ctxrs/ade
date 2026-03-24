use super::*;

pub(super) async fn normalize_message_attachments(
    state: &Arc<AppState>,
    attachments: Vec<MessageAttachment>,
) -> Result<Vec<MessageAttachment>, StatusCode> {
    let mut out = Vec::with_capacity(attachments.len());
    for att in attachments {
        match att {
            MessageAttachment::Image {
                mime_type,
                data_base64,
                name,
            } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data_base64.as_bytes())
                    .map_err(|_| StatusCode::BAD_REQUEST)?;
                let saved =
                    persist_blob_bytes(state.as_ref(), &bytes, &mime_type, name.as_deref()).await?;
                out.push(MessageAttachment::ImageRef {
                    blob_id: saved.blob_id,
                    mime_type,
                    name,
                });
            }
            MessageAttachment::ImageRef {
                blob_id,
                mime_type,
                name,
            } => {
                let exists = state
                    .global_store()
                    .get_blob(&blob_id)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                    .is_some();
                if !exists {
                    return Err(StatusCode::BAD_REQUEST);
                }
                out.push(MessageAttachment::ImageRef {
                    blob_id,
                    mime_type,
                    name,
                });
            }
        }
    }
    Ok(out)
}

#[derive(Debug, PartialEq, Eq)]
struct AttachmentSignature {
    mime_type: String,
    name: Option<String>,
    sha256: String,
}

async fn attachment_signature(
    state: &Arc<AppState>,
    attachment: &MessageAttachment,
) -> Result<AttachmentSignature, StatusCode> {
    match attachment {
        MessageAttachment::Image {
            mime_type,
            data_base64,
            name,
        } => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data_base64.as_bytes())
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            let mut hasher = sha2::Sha256::new();
            hasher.update(&bytes);
            let sha256 = hex::encode(hasher.finalize());
            Ok(AttachmentSignature {
                mime_type: mime_type.clone(),
                name: name.clone(),
                sha256,
            })
        }
        MessageAttachment::ImageRef {
            blob_id,
            mime_type,
            name,
        } => {
            let blob = state
                .global_store()
                .get_blob(blob_id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let Some((sha256, _mime_type, _bytes, _name, _created_at)) = blob else {
                return Err(StatusCode::BAD_REQUEST);
            };
            Ok(AttachmentSignature {
                mime_type: mime_type.clone(),
                name: name.clone(),
                sha256,
            })
        }
    }
}

pub(super) async fn attachments_match(
    state: &Arc<AppState>,
    existing: &[MessageAttachment],
    requested: &[MessageAttachment],
) -> Result<bool, StatusCode> {
    if existing.len() != requested.len() {
        return Ok(false);
    }
    let mut existing_sig = Vec::with_capacity(existing.len());
    for attachment in existing {
        existing_sig.push(attachment_signature(state, attachment).await?);
    }
    let mut requested_sig = Vec::with_capacity(requested.len());
    for attachment in requested {
        requested_sig.push(attachment_signature(state, attachment).await?);
    }
    Ok(existing_sig == requested_sig)
}

pub(super) fn delivery_matches(left: &MessageDelivery, right: &MessageDelivery) -> bool {
    std::mem::discriminant(left) == std::mem::discriminant(right)
}

pub(crate) async fn ensure_session_turn_for_message(
    store: &ctx_store::Store,
    session_id: SessionId,
    turn_id: TurnId,
    message: &Message,
) -> Result<(), StatusCode> {
    let existing_turn = store
        .get_session_turn_by_id(turn_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(existing) = existing_turn {
        let matches =
            existing.session_id == session_id && existing.user_message_id == Some(message.id);
        if !matches {
            return Err(StatusCode::CONFLICT);
        }
        return Ok(());
    }

    let turn_status = if matches!(message.delivery, MessageDelivery::Queued) {
        SessionTurnStatus::Queued
    } else {
        SessionTurnStatus::Running
    };
    let turn = SessionTurn {
        turn_id,
        session_id,
        run_id: message.run_id,
        user_message_id: Some(message.id),
        status: turn_status,
        start_seq: None,
        end_seq: None,
        started_at: message.created_at,
        updated_at: message.created_at,
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
                existing.session_id == session_id && existing.user_message_id == Some(message.id);
            if !matches {
                return Err(StatusCode::CONFLICT);
            }
        } else {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
    Ok(())
}

pub(crate) async fn delete_session_message(
    State(state): State<Arc<AppState>>,
    Path((session_id, id)): Path<(String, String)>,
) -> Result<StatusCode, StatusCode> {
    let session_id =
        SessionId(uuid::Uuid::parse_str(&session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let msg_id = MessageId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
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

pub(crate) async fn post_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<PostMessageReq>,
) -> Result<Json<Message>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = {
        const STORE_OPEN_RETRY_LIMIT: usize = 3;
        const STORE_OPEN_RETRY_BASE_MS: u64 = 40;
        let mut attempt = 0usize;
        loop {
            match state.store_for_session(session_id).await {
                Ok(store) => break store,
                Err(err) => {
                    let msg = err.to_string().to_lowercase();
                    let transient = msg.contains("database is locked")
                        || msg.contains("sqlite_busy")
                        || msg.contains("database is busy");
                    if transient && attempt < STORE_OPEN_RETRY_LIMIT {
                        attempt += 1;
                        let backoff_ms = STORE_OPEN_RETRY_BASE_MS.saturating_mul(attempt as u64);
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                        continue;
                    }
                    if msg.contains("workspace missing for session") {
                        return Err(StatusCode::NOT_FOUND);
                    }
                    tracing::warn!(session_id = %session_id.0, "store_for_session failed: {err:#}");
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            }
        }
    };
    let run_id_header = headers
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    state.remember_session_meta(&session).await;

    let requested_delivery = req.delivery.clone();
    let delivery = match requested_delivery.clone() {
        Some(d) => d,
        None => {
            if state.is_running(session_id).await {
                MessageDelivery::Queued
            } else {
                MessageDelivery::Immediate
            }
        }
    };

    let (message_id, turn_id, client_ids) = match (
        req.id.as_deref().map(str::trim).filter(|v| !v.is_empty()),
        req.turn_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty()),
    ) {
        (Some(message_id), Some(turn_id)) => (
            MessageId(uuid::Uuid::parse_str(message_id).map_err(|_| StatusCode::BAD_REQUEST)?),
            TurnId(uuid::Uuid::parse_str(turn_id).map_err(|_| StatusCode::BAD_REQUEST)?),
            true,
        ),
        (None, None) => (MessageId::new(), TurnId::new(), false),
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    let attachments = normalize_message_attachments(&state, req.attachments).await?;
    let content = req.content;

    if client_ids {
        if let Some(existing) = store
            .get_message(message_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
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
                ensure_session_turn_for_message(&store, session_id, turn_id, &existing).await?;
                return Ok(Json(existing));
            }
            return Err(StatusCode::CONFLICT);
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
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            };
            let Some(existing) = store
                .get_message(message_id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            else {
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
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
                return Err(StatusCode::CONFLICT);
            }
        }
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let start_seq = event.seq;

    let turn_status = if matches!(saved.delivery, MessageDelivery::Queued) {
        SessionTurnStatus::Queued
    } else {
        SessionTurnStatus::Running
    };
    let turn = SessionTurn {
        turn_id,
        session_id,
        run_id: Some(run_id),
        user_message_id: Some(saved.id),
        status: turn_status,
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
    let existing_turn = store
        .get_session_turn_by_id(turn_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(existing) = existing_turn {
        let matches =
            existing.session_id == session_id && existing.user_message_id == Some(saved.id);
        if !matches {
            return Err(StatusCode::CONFLICT);
        }
    } else if let Err(err) = store.insert_session_turn(turn).await {
        if !is_unique_constraint_violation(&err) {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        let existing = store
            .get_session_turn_by_id(turn_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(existing) = existing {
            let matches =
                existing.session_id == session_id && existing.user_message_id == Some(saved.id);
            if !matches {
                return Err(StatusCode::CONFLICT);
            }
        } else {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
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
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
