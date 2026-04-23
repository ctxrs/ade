use std::sync::Arc;

use ctx_core::ids::SessionId;
use ctx_core::models::{
    Message, MessageAttachment, MessageDelivery, MessageRole, Session, SessionActivityState,
    SessionEvent, SessionEventType, SessionSummaryDelta, SessionTurn, SessionTurnStatus,
    SessionTurnToolSummary,
};

use crate::daemon::state::AppState;

pub(super) fn message_from_event(event: &SessionEvent, session: &Session) -> Option<Message> {
    let message_id = event
        .payload_json
        .get("message_id")
        .and_then(|value| value.as_str())
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .map(ctx_core::ids::MessageId)?;
    let content = event
        .payload_json
        .get("content")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())?;
    let delivery = event
        .payload_json
        .get("delivery")
        .and_then(|value| serde_json::from_value::<MessageDelivery>(value.clone()).ok())
        .unwrap_or(MessageDelivery::Immediate);
    let attachments = event
        .payload_json
        .get("attachments")
        .and_then(|value| serde_json::from_value::<Vec<MessageAttachment>>(value.clone()).ok())
        .unwrap_or_default();
    let order_seq = event
        .payload_json
        .get("order_seq")
        .or_else(|| event.payload_json.get("orderSeq"))
        .and_then(|value| value.as_i64());
    let role = match event.event_type {
        SessionEventType::UserMessage => MessageRole::User,
        SessionEventType::AssistantMessageInserted => MessageRole::Assistant,
        _ => return None,
    };
    let delivered_at = match role {
        MessageRole::Assistant => Some(event.created_at),
        _ => None,
    };
    Some(Message {
        id: message_id,
        session_id: event.session_id,
        task_id: session.task_id,
        run_id: event.run_id,
        turn_id: event.turn_id,
        turn_sequence: event
            .payload_json
            .get("turn_sequence")
            .and_then(|value| value.as_i64()),
        order_seq,
        role,
        content,
        attachments,
        delivery,
        delivered_at,
        created_at: event.created_at,
    })
}

pub(super) fn derive_message_preview(content: &str) -> String {
    let trimmed = content.trim();
    let line = trimmed.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return String::new();
    }
    const MAX_CHARS: usize = 160;
    let mut out: String = line.chars().take(MAX_CHARS).collect();
    if line.chars().count() > MAX_CHARS {
        out.push_str("...");
    }
    out
}

pub(super) fn event_context_window(event: &SessionEvent) -> Option<serde_json::Value> {
    event.payload_json.get("context_window").cloned()
}

pub(super) fn is_session_gap_notice(event: &SessionEvent) -> bool {
    matches!(event.event_type, SessionEventType::Notice)
        && event
            .payload_json
            .get("kind")
            .and_then(|value| value.as_str())
            .is_some_and(|kind| kind == "session_gap")
}

fn turn_status_from_finished_event(event: &SessionEvent) -> SessionTurnStatus {
    event
        .payload_json
        .get("status")
        .and_then(|value| serde_json::from_value::<SessionTurnStatus>(value.clone()).ok())
        .unwrap_or(SessionTurnStatus::Completed)
}

pub(super) fn patch_turn_from_event(turn: &mut SessionTurn, event: &SessionEvent) {
    match event.event_type {
        SessionEventType::TurnQueued => {
            turn.status = SessionTurnStatus::Queued;
        }
        SessionEventType::TurnStarted => {
            turn.status = SessionTurnStatus::Running;
        }
        SessionEventType::TurnFinished => {
            turn.status = turn_status_from_finished_event(event);
            turn.end_seq = Some(event.seq);
        }
        _ => {}
    }
    if let Some(metrics_json) = event_context_window(event) {
        turn.metrics_json = Some(metrics_json);
    }
    turn.updated_at = event.created_at;
}

pub(super) fn derive_summary_activity(event: &SessionEvent) -> Option<SessionActivityState> {
    match event.event_type {
        SessionEventType::TurnQueued => Some(SessionActivityState {
            is_working: false,
            last_turn_status: Some(SessionTurnStatus::Queued),
        }),
        SessionEventType::TurnStarted => Some(SessionActivityState {
            is_working: true,
            last_turn_status: Some(SessionTurnStatus::Running),
        }),
        SessionEventType::TurnFinished => Some(SessionActivityState {
            is_working: false,
            last_turn_status: Some(turn_status_from_finished_event(event)),
        }),
        _ => None,
    }
}

pub(super) fn activity_from_turn(turn: &SessionTurn) -> SessionActivityState {
    match turn.status {
        SessionTurnStatus::Queued => SessionActivityState {
            is_working: false,
            last_turn_status: Some(SessionTurnStatus::Queued),
        },
        SessionTurnStatus::Starting => SessionActivityState {
            is_working: true,
            last_turn_status: Some(SessionTurnStatus::Starting),
        },
        SessionTurnStatus::Running => SessionActivityState {
            is_working: true,
            last_turn_status: Some(SessionTurnStatus::Running),
        },
        SessionTurnStatus::Completed => SessionActivityState {
            is_working: false,
            last_turn_status: Some(SessionTurnStatus::Completed),
        },
        SessionTurnStatus::Interrupted => SessionActivityState {
            is_working: false,
            last_turn_status: Some(SessionTurnStatus::Interrupted),
        },
        SessionTurnStatus::Failed => SessionActivityState {
            is_working: false,
            last_turn_status: Some(SessionTurnStatus::Failed),
        },
    }
}

pub(super) fn should_include_session_metadata_in_head_delta(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::Init | SessionEventType::Notice
    )
}

pub(super) fn recompute_turn_tool_counts(
    turn: &mut SessionTurn,
    tool_summaries: &[SessionTurnToolSummary],
) {
    let mut total = 0_i64;
    let mut pending = 0_i64;
    let mut running = 0_i64;
    let mut completed = 0_i64;
    let mut failed = 0_i64;
    for summary in tool_summaries
        .iter()
        .filter(|summary| summary.turn_id == turn.turn_id)
    {
        total += 1;
        match summary.status.as_deref() {
            Some("running") | Some("in_progress") => running += 1,
            Some("completed") | Some("complete") | Some("ok") | Some("succeeded") => {
                completed += 1;
            }
            Some("failed") | Some("error") => failed += 1,
            _ => pending += 1,
        }
    }
    turn.tool_total = total;
    turn.tool_pending = pending;
    turn.tool_running = running;
    turn.tool_completed = completed;
    turn.tool_failed = failed;
}

pub(super) fn build_session_summary_delta(
    session: &Session,
    activity: Option<SessionActivityState>,
    last_message_at: Option<chrono::DateTime<chrono::Utc>>,
    last_message_preview: Option<String>,
    last_event_seq: i64,
    projection_rev: i64,
    state_rev: i64,
) -> Option<SessionSummaryDelta> {
    if activity.is_none() && last_message_at.is_none() && last_message_preview.is_none() {
        return None;
    }

    Some(SessionSummaryDelta {
        session_id: session.id,
        task_id: session.task_id,
        activity,
        last_message_at,
        last_message_preview,
        last_event_seq: Some(last_event_seq),
        projection_rev: Some(projection_rev),
        state_rev: Some(state_rev),
        emitted_at_ms: Some(chrono::Utc::now().timestamp_millis()),
    })
}

pub(super) async fn resolve_projection_rev_for_stream_delta<F, Fut>(
    stream_only: bool,
    last_event_seq: i64,
    cached_projection_rev: i64,
    load_projection_rev: F,
) -> i64
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Option<i64>>,
{
    if stream_only {
        return cached_projection_rev.max(0);
    }
    load_projection_rev().await.unwrap_or(last_event_seq.max(0))
}

pub(super) fn turn_from_event(
    event: &SessionEvent,
    message: Option<&Message>,
) -> Option<SessionTurn> {
    if !matches!(event.event_type, SessionEventType::UserMessage) {
        return None;
    }
    let turn_id = event.turn_id?;
    let delivery = message
        .map(|msg| msg.delivery.clone())
        .unwrap_or(MessageDelivery::Immediate);
    let status = if matches!(delivery, MessageDelivery::Queued) {
        SessionTurnStatus::Queued
    } else {
        SessionTurnStatus::Starting
    };
    Some(SessionTurn {
        turn_id,
        session_id: event.session_id,
        run_id: event.run_id,
        user_message_id: message.map(|msg| msg.id),
        status,
        start_seq: Some(event.seq),
        end_seq: None,
        started_at: event.created_at,
        updated_at: event.created_at,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    })
}

pub(super) fn should_refresh_turn_from_store(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::TurnQueued
            | SessionEventType::TurnStarted
            | SessionEventType::Done
            | SessionEventType::TurnFinished
            | SessionEventType::TurnInterrupted
            | SessionEventType::Error
    )
}

pub(super) async fn turn_from_cached_head_for_read(
    state: &Arc<AppState>,
    session_id: SessionId,
    turn_id: ctx_core::ids::TurnId,
) -> Option<SessionTurn> {
    state
        .workspaces
        .workspace_active_snapshot
        .get_cached_session_head_for_read(session_id)
        .await
        .and_then(|head| head.turns.into_iter().find(|turn| turn.turn_id == turn_id))
}
