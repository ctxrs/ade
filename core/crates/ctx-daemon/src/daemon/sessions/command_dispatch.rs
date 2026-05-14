use std::sync::Arc;
use std::time::Instant;

use ctx_core::ids::{MessageId, SessionId};
use ctx_core::models::{Message, MessageDelivery, Session, SessionEventType};
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};
use ctx_session_tools::interrupt_telemetry::{metric_labels, InterruptTelemetryContext};
use ctx_store::Store;

use crate::daemon::scheduler::{QueuedMessage, SchedulerCommand};
use crate::daemon::sessions::title_generation::schedule_session_title_generation;
use crate::daemon::{DaemonState, SessionStoreAccessError};

#[derive(Debug)]
pub enum SessionSchedulerCommandError {
    BadRequest,
    NotFound,
    StoreUnavailable,
}

pub async fn cancel_session(
    state: &Arc<DaemonState>,
    session_id: SessionId,
) -> Result<(), SessionSchedulerCommandError> {
    let (_store, session) = load_session_for_command(state, session_id).await?;
    let tx = state.ensure_scheduler(session).await;
    let _ = tx.send(SchedulerCommand::Cancel).await;
    Ok(())
}

pub async fn interrupt_session(
    state: &Arc<DaemonState>,
    session_id: SessionId,
    request_started: Instant,
) -> Result<(), SessionSchedulerCommandError> {
    let (store, session) = load_session_for_command(state, session_id).await?;
    let session_root_kind = match store.get_worktree(session.worktree_id).await {
        Ok(Some(worktree)) if worktree.vcs_ref.is_some() || worktree.git_branch.is_some() => {
            "worktree"
        }
        Ok(Some(_)) => "workspace_root",
        _ => "unknown",
    };
    let provider_id = session.provider_id.clone();
    let model_id = session.model_id.clone();
    let execution_environment = session.execution_environment;
    let tx = state.ensure_scheduler(session).await;
    let interrupt = InterruptTelemetryContext::new(uuid::Uuid::new_v4().to_string());
    let _ = tx
        .send(SchedulerCommand::Interrupt(interrupt.clone()))
        .await;
    let dispatch_ms = request_started.elapsed().as_millis() as u64;
    let metric = PerfMetric {
        name: "scheduler.interrupt_http_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: dispatch_ms as f64,
        labels: metric_labels(
            &provider_id,
            &model_id,
            execution_environment.as_str(),
            session_root_kind,
            "http_dispatch",
        ),
    };
    state
        .telemetry
        .perf_telemetry
        .record_metric(metric, None, None, None)
        .await;
    tracing::info!(
        session_id = %session_id.0,
        interrupt_id = %interrupt.interrupt_id(),
        provider_id = %provider_id,
        model_id = %model_id,
        dispatch_ms,
        "session interrupt dispatched"
    );
    Ok(())
}

pub async fn delete_queued_session_message(
    state: &Arc<DaemonState>,
    session_id: SessionId,
    message_id: MessageId,
) -> Result<(), SessionSchedulerCommandError> {
    let store = state
        .existing_session_store_for_write(session_id)
        .await
        .map_err(session_store_error)?;
    let msg = store
        .get_message(message_id)
        .await
        .map_err(|_| SessionSchedulerCommandError::StoreUnavailable)?
        .ok_or(SessionSchedulerCommandError::NotFound)?;
    if msg.session_id != session_id {
        return Err(SessionSchedulerCommandError::NotFound);
    }

    if !matches!(msg.delivery, MessageDelivery::Queued) || msg.delivered_at.is_some() {
        return Err(SessionSchedulerCommandError::BadRequest);
    }
    store
        .delete_message(message_id)
        .await
        .map_err(|_| SessionSchedulerCommandError::StoreUnavailable)?;
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
        .map_err(|_| SessionSchedulerCommandError::StoreUnavailable)?;
    state.publish_event(removed).await;

    if let Some(tx) = state.session_scheduler_sender(msg.session_id).await {
        let _ = tx.send(SchedulerCommand::RemoveQueued(message_id)).await;
    }
    Ok(())
}

pub async fn enqueue_user_message_for_scheduler(
    state: &Arc<DaemonState>,
    store: &Store,
    session: Session,
    message: Message,
    run_id_header: Option<String>,
) {
    let tx = state.ensure_scheduler(session.clone()).await;
    let queued = QueuedMessage {
        message: message.clone(),
        enqueued_at: Instant::now(),
        run_id: run_id_header,
    };
    let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

    if let Ok(count) = store.count_user_messages_for_session(session.id).await {
        if count == 1 {
            let _ =
                schedule_session_title_generation(state.clone(), session, message.content, false)
                    .await;
        }
    }
}

async fn load_session_for_command(
    state: &Arc<DaemonState>,
    session_id: SessionId,
) -> Result<(Store, Session), SessionSchedulerCommandError> {
    let store = state
        .existing_session_store_for_write(session_id)
        .await
        .map_err(session_store_error)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| SessionSchedulerCommandError::StoreUnavailable)?
        .ok_or(SessionSchedulerCommandError::NotFound)?;
    Ok((store, session))
}

fn session_store_error(error: SessionStoreAccessError) -> SessionSchedulerCommandError {
    match error {
        SessionStoreAccessError::NotFound => SessionSchedulerCommandError::NotFound,
        SessionStoreAccessError::LookupUnavailable(_)
        | SessionStoreAccessError::StoreUnavailable => {
            SessionSchedulerCommandError::StoreUnavailable
        }
    }
}
