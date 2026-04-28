use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use ctx_core::ids::{RunId, SessionId, TurnId};
use ctx_core::models::{Message, MessageDelivery, MessageRole, SessionEvent, SessionEventType};

use crate::daemon::AppState;

const STORE_WRITE_RETRY_LIMIT: usize = 3;
const STORE_WRITE_RETRY_BASE_MS: u64 = 40;

fn is_transient_store_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("database is locked")
        || msg.contains("sqlite_busy")
        || msg.contains("database is busy")
}

fn maybe_fail_persist_assistant_message() -> Result<()> {
    if let Err(err) =
        crate::fault_injection::maybe_fail("ctx_http.persist_assistant_message.transient")
    {
        return Err(anyhow!("database is locked (fault injection): {err}"));
    }
    crate::fault_injection::maybe_fail("ctx_http.persist_assistant_message.fatal")
}

pub(crate) async fn append_session_event_with_retry(
    store: &ctx_store::Store,
    session_id: ctx_core::ids::SessionId,
    run_id: Option<RunId>,
    turn_id: Option<TurnId>,
    event_type: SessionEventType,
    payload_json: serde_json::Value,
) -> Result<SessionEvent> {
    let mut attempt = 0usize;
    loop {
        match store
            .append_session_event(
                session_id,
                run_id,
                turn_id,
                event_type.clone(),
                payload_json.clone(),
            )
            .await
        {
            Ok(event) => return Ok(event),
            Err(err) => {
                if !is_transient_store_error(&err) || attempt >= STORE_WRITE_RETRY_LIMIT {
                    return Err(err);
                }
                attempt += 1;
                let backoff_ms = STORE_WRITE_RETRY_BASE_MS.saturating_mul(attempt as u64);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
        }
    }
}

pub(crate) async fn claim_session_provider_session_ref_with_retry(
    store: &ctx_store::Store,
    session_id: SessionId,
    provider_session_ref: String,
    source: &str,
) -> Result<()> {
    let mut attempt = 0usize;
    loop {
        match store
            .claim_session_provider_session_ref(session_id, provider_session_ref.clone(), source)
            .await
        {
            Ok(()) => return Ok(()),
            Err(err) => {
                if !is_transient_store_error(&err) || attempt >= STORE_WRITE_RETRY_LIMIT {
                    return Err(err);
                }
                attempt += 1;
                let backoff_ms = STORE_WRITE_RETRY_BASE_MS.saturating_mul(attempt as u64);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
        }
    }
}

pub(crate) async fn emit_event(
    state: &Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
    run_id: Option<RunId>,
    turn_id: Option<TurnId>,
    event_type: SessionEventType,
    payload_json: serde_json::Value,
) -> Result<SessionEvent> {
    let store = state.store_for_session(session_id).await?;
    let event = append_session_event_with_retry(
        &store,
        session_id,
        run_id,
        turn_id,
        event_type,
        payload_json,
    )
    .await?;
    state.publish_event(event.clone()).await;
    Ok(event)
}

pub(crate) async fn flush_session_events(
    store: &ctx_store::Store,
    session_id: ctx_core::ids::SessionId,
    context: &str,
) {
    if let Err(err) = store.flush_session_event_log().await {
        tracing::warn!(
            session_id = %session_id.0,
            context,
            "failed to flush session event log: {err:#}"
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_assistant_message(
    _state: &AppState,
    store: &ctx_store::Store,
    _workspace_id: ctx_core::ids::WorkspaceId,
    message_id: ctx_core::ids::MessageId,
    order_seq: i64,
    session_id: ctx_core::ids::SessionId,
    task_id: ctx_core::ids::TaskId,
    run_id: RunId,
    turn_id: TurnId,
    content: String,
    turn_sequence: i64,
    created_at: chrono::DateTime<chrono::Utc>,
) -> Result<Message> {
    if content.is_empty() {
        return Err(anyhow!("assistant message content empty"));
    }
    let msg = Message {
        id: message_id,
        session_id,
        task_id,
        run_id: Some(run_id),
        turn_id: Some(turn_id),
        turn_sequence: Some(turn_sequence),
        order_seq: Some(order_seq),
        role: MessageRole::Assistant,
        content,
        attachments: vec![],
        delivery: MessageDelivery::Immediate,
        delivered_at: Some(created_at),
        created_at,
    };
    let mut attempt = 0usize;
    loop {
        if let Err(err) = maybe_fail_persist_assistant_message() {
            if !is_transient_store_error(&err) || attempt >= STORE_WRITE_RETRY_LIMIT {
                tracing::warn!("assistant message insert failed: {err:#}");
                return Err(err);
            }
            attempt += 1;
            let backoff_ms = STORE_WRITE_RETRY_BASE_MS.saturating_mul(attempt as u64);
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            continue;
        }
        match store.insert_message(msg.clone()).await {
            Ok(saved) => return Ok(saved),
            Err(err) => {
                if !is_transient_store_error(&err) || attempt >= STORE_WRITE_RETRY_LIMIT {
                    tracing::warn!("assistant message insert failed: {err:#}");
                    return Err(err);
                }
                attempt += 1;
                let backoff_ms = STORE_WRITE_RETRY_BASE_MS.saturating_mul(attempt as u64);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
        }
    }
}
