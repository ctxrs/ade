use ctx_core::ids::MessageId;
use ctx_core::models::{SessionEvent, SessionEventType};
use serde_json::{json, Value};

use crate::order_seq::{attach_order_seq, read_order_seq};
use crate::scheduler::persistence::{emit_event, persist_assistant_message};
use crate::storage_guard;

use super::super::helpers::strip_emitted_prefix;
use super::failure::{fail_turn, TurnFailurePayload};
use super::{EventLoopRuntimeState, TurnEventLoop};

pub(super) async fn handle_assistant_complete(
    ctx: &TurnEventLoop,
    runtime: &mut EventLoopRuntimeState,
    event: &SessionEvent,
) {
    let provider_message_id = event
        .payload_json
        .get("message_id")
        .or_else(|| event.payload_json.get("messageId"))
        .and_then(Value::as_str)
        .map(|s: &str| s.to_string())
        .or_else(|| runtime.assistant_partial_message_id.clone());
    let order_seq = read_order_seq(&event.payload_json).unwrap_or_else(|| {
        tracing::warn!(
            session_id = %ctx.session_id.0,
            run_id = %ctx.run_id.0,
            turn_id = %ctx.turn_id.0,
            "assistant_complete missing order_seq; assigning fallback message order"
        );
        0
    });
    let content: Option<String> = event
        .payload_json
        .get("full_content")
        .or_else(|| event.payload_json.get("content"))
        .and_then(Value::as_str)
        .map(|s: &str| s.to_string())
        .or_else(|| {
            if runtime.assistant_partial.is_empty() {
                None
            } else {
                Some(runtime.assistant_partial.clone())
            }
        });

    if let Some(content) = content {
        if let Some(content) = strip_emitted_prefix(&content, &runtime.assistant_emitted) {
            let assistant_message_id = MessageId::new();
            let persisted_order_seq = if order_seq > 0 {
                order_seq
            } else {
                let mut order_seq_state = ctx.order_seq_state.lock().await;
                order_seq_state.get_or_assign(format!("message:{}", assistant_message_id.0), None)
            };
            match persist_assistant_message(
                ctx.state.as_ref(),
                &ctx.store,
                ctx.workspace_id,
                assistant_message_id,
                persisted_order_seq,
                ctx.session_id,
                ctx.task_id,
                ctx.run_id,
                ctx.turn_id,
                content,
                runtime.assistant_sequence + 1,
                event.created_at,
            )
            .await
            {
                Ok(saved) => {
                    runtime.assistant_sequence += 1;
                    runtime.assistant_emitted.push_str(&saved.content);
                    runtime.assistant_partial.clear();
                    let mut payload = json!({
                        "message_id": saved.id.0,
                        "content": saved.content,
                        "delivery": saved.delivery,
                        "attachments": saved.attachments,
                        "turn_sequence": saved.turn_sequence,
                        "order_seq": saved.order_seq,
                    });
                    if let Some(provider_message_id) = provider_message_id {
                        if let Some(obj) = payload.as_object_mut() {
                            obj.insert(
                                "provider_message_id".to_string(),
                                json!(provider_message_id),
                            );
                        }
                    }
                    {
                        let mut order_seq_state = ctx.order_seq_state.lock().await;
                        attach_order_seq(
                            &mut order_seq_state,
                            &SessionEventType::AssistantMessageInserted,
                            &mut payload,
                            Some(&ctx.turn_id),
                            runtime.assistant_sequence,
                        );
                    }
                    let _ = emit_event(
                        &ctx.state,
                        ctx.session_id,
                        Some(ctx.run_id),
                        Some(ctx.turn_id),
                        SessionEventType::AssistantMessageInserted,
                        payload,
                    )
                    .await;
                    runtime.assistant_partial_message_id = None;
                }
                Err(err) => {
                    let err_string = format!("{err:#}");
                    let is_storage_exhausted =
                        storage_guard::is_storage_exhaustion_error(&err_string);
                    let details = Some(json!({
                        "provider_message_id": provider_message_id,
                        "order_seq": persisted_order_seq,
                        "turn_sequence": runtime.assistant_sequence + 1,
                        "root_cause": err_string,
                    }));
                    let storage_status = ctx.state.storage_guard_snapshot();
                    fail_turn(
                        ctx,
                        runtime,
                        TurnFailurePayload {
                            error_message: if is_storage_exhausted {
                                storage_guard::storage_exhaustion_message(
                                    storage_status.active.as_ref(),
                                )
                            } else {
                                format!("failed to persist assistant message: {err:#}")
                            },
                            details,
                            kind: Some(json!(if is_storage_exhausted {
                                "storage_exhausted"
                            } else {
                                "assistant_message_persist_failed"
                            })),
                        },
                        true,
                    )
                    .await;
                    runtime.assistant_partial.clear();
                    runtime.assistant_partial_message_id = None;
                }
            }
        } else {
            runtime.assistant_partial.clear();
            runtime.assistant_partial_message_id = None;
        }
    }

    let _ = ctx
        .store
        .delete_session_events_for_turn_types(
            ctx.session_id,
            ctx.turn_id,
            &[
                SessionEventType::AssistantChunk,
                SessionEventType::ThoughtChunk,
            ],
        )
        .await;
}
