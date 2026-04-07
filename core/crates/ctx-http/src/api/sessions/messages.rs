use super::*;

type ApiErr = (StatusCode, Json<ApiErrorResp>);

const MAX_MESSAGE_IMAGE_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;
const MAX_MESSAGE_IMAGE_ATTACHMENT_MIB: usize = MAX_MESSAGE_IMAGE_ATTACHMENT_BYTES / (1024 * 1024);

fn api_error(status: StatusCode, error: impl Into<String>) -> ApiErr {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}

fn image_attachment_too_large_error() -> ApiErr {
    api_error(
        StatusCode::PAYLOAD_TOO_LARGE,
        format!("Image attachments must be {MAX_MESSAGE_IMAGE_ATTACHMENT_MIB} MiB or smaller."),
    )
}

fn ensure_image_attachment_size(bytes: usize) -> Result<(), ApiErr> {
    if bytes > MAX_MESSAGE_IMAGE_ATTACHMENT_BYTES {
        return Err(image_attachment_too_large_error());
    }
    Ok(())
}

fn session_store_api_error(status: StatusCode) -> ApiErr {
    match status {
        StatusCode::NOT_FOUND => api_error(StatusCode::NOT_FOUND, "Session not found."),
        _ => api_error(status, "Failed to open session."),
    }
}

fn ensure_image_attachment_mime_type(mime_type: &str) -> Result<(), ApiErr> {
    if mime_type
        .trim()
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("image/"))
    {
        return Ok(());
    }
    Err(api_error(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "Only image attachments are supported.",
    ))
}

fn decoded_base64_len(data_base64: &str) -> Result<usize, ApiErr> {
    let bytes = data_base64.as_bytes();
    if bytes.is_empty() {
        return Ok(0);
    }
    if !bytes.len().is_multiple_of(4) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Invalid image attachment.",
        ));
    }
    let padding = if bytes.ends_with(b"==") {
        2
    } else if bytes.ends_with(b"=") {
        1
    } else {
        0
    };
    Ok((bytes.len() / 4) * 3 - padding)
}

fn decode_inline_image_attachment(data_base64: &str) -> Result<Vec<u8>, ApiErr> {
    let decoded_len = decoded_base64_len(data_base64)?;
    ensure_image_attachment_size(decoded_len)?;
    base64::engine::general_purpose::STANDARD
        .decode(data_base64.as_bytes())
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "Invalid image attachment."))
}

struct ImageBlobMetadata {
    sha256: String,
    mime_type: String,
}

async fn load_image_blob_metadata(
    state: &Arc<AppState>,
    blob_id: &str,
) -> Result<ImageBlobMetadata, ApiErr> {
    let blob = state.global_store().get_blob(blob_id).await.map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to inspect image attachment.",
        )
    })?;
    let Some((sha256, stored_mime_type, bytes, _stored_name, _created_at)) = blob else {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "Image attachment blob was not found.",
        ));
    };
    ensure_image_attachment_mime_type(&stored_mime_type)?;
    let bytes = usize::try_from(bytes).map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Invalid image attachment metadata.",
        )
    })?;
    ensure_image_attachment_size(bytes)?;
    Ok(ImageBlobMetadata {
        sha256,
        mime_type: stored_mime_type,
    })
}

pub(super) async fn normalize_message_attachments(
    state: &Arc<AppState>,
    attachments: Vec<MessageAttachment>,
) -> Result<Vec<MessageAttachment>, ApiErr> {
    let mut out = Vec::with_capacity(attachments.len());
    for att in attachments {
        match att {
            MessageAttachment::Image {
                mime_type,
                data_base64,
                name,
            } => {
                let bytes = decode_inline_image_attachment(&data_base64)?;
                let saved = persist_blob_bytes(state.as_ref(), &bytes, &mime_type, name.as_deref())
                    .await
                    .map_err(|status| match status {
                        StatusCode::PAYLOAD_TOO_LARGE => image_attachment_too_large_error(),
                        StatusCode::UNSUPPORTED_MEDIA_TYPE => api_error(
                            StatusCode::UNSUPPORTED_MEDIA_TYPE,
                            "Only image attachments are supported.",
                        ),
                        _ => api_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Failed to persist image attachment.",
                        ),
                    })?;
                out.push(MessageAttachment::ImageRef {
                    blob_id: saved.blob_id,
                    mime_type,
                    name,
                });
            }
            MessageAttachment::ImageRef { blob_id, name, .. } => {
                let metadata = load_image_blob_metadata(state, &blob_id).await?;
                out.push(MessageAttachment::ImageRef {
                    blob_id,
                    mime_type: metadata.mime_type,
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
) -> Result<AttachmentSignature, ApiErr> {
    match attachment {
        MessageAttachment::Image {
            mime_type,
            data_base64,
            name,
        } => {
            let bytes = decode_inline_image_attachment(data_base64)?;
            let mut hasher = sha2::Sha256::new();
            hasher.update(&bytes);
            let sha256 = hex::encode(hasher.finalize());
            Ok(AttachmentSignature {
                mime_type: mime_type.clone(),
                name: name.clone(),
                sha256,
            })
        }
        MessageAttachment::ImageRef { blob_id, name, .. } => {
            let metadata = load_image_blob_metadata(state, blob_id).await?;
            Ok(AttachmentSignature {
                mime_type: metadata.mime_type,
                name: name.clone(),
                sha256: metadata.sha256,
            })
        }
    }
}

pub(super) async fn attachments_match(
    state: &Arc<AppState>,
    existing: &[MessageAttachment],
    requested: &[MessageAttachment],
) -> Result<bool, ApiErr> {
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
