use ctx_core::models::{
    Message, MessageAttachment, MessageDelivery, MessageRole, Session, SessionActivityState,
    SessionEvent, SessionEventType, SessionSummaryDelta, SessionTurn, SessionTurnStatus,
    SessionTurnToolSummary,
};

pub fn message_from_event(event: &SessionEvent, session: &Session) -> Option<Message> {
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

pub fn derive_message_preview(content: &str) -> String {
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

pub fn event_context_window(event: &SessionEvent) -> Option<serde_json::Value> {
    event.payload_json.get("context_window").cloned()
}

pub fn is_session_gap_notice(event: &SessionEvent) -> bool {
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

pub fn patch_turn_from_event(turn: &mut SessionTurn, event: &SessionEvent) {
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

pub fn derive_summary_activity(event: &SessionEvent) -> Option<SessionActivityState> {
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

pub fn activity_from_turn(turn: &SessionTurn) -> SessionActivityState {
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

pub fn should_include_session_metadata_in_head_delta(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::Init | SessionEventType::Notice
    )
}

pub fn recompute_turn_tool_counts(
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

pub fn build_session_summary_delta(
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

pub async fn resolve_projection_rev_for_stream_delta<F, Fut>(
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

pub fn turn_from_event(event: &SessionEvent, message: Option<&Message>) -> Option<SessionTurn> {
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

pub fn should_refresh_turn_from_store(event_type: &SessionEventType) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ctx_core::ids::{SessionEventId, SessionId, TaskId, WorkspaceId, WorktreeId};
    use ctx_core::models::{
        ExecutionEnvironment, SessionEventType, SessionStatus, SessionTurnStatus,
    };
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn test_session() -> Session {
        Session {
            id: SessionId::new(),
            task_id: TaskId::new(),
            workspace_id: WorkspaceId::new(),
            worktree_id: WorktreeId::new(),
            execution_environment: ExecutionEnvironment::Host,
            parent_session_id: None,
            relationship: None,
            provider_id: "fake".to_string(),
            model_id: "fake-model".to_string(),
            reasoning_effort: None,
            title: String::new(),
            agent_role: "assistant".to_string(),
            status: SessionStatus::Active,
            provider_session_ref: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn test_event(event_type: SessionEventType, payload_json: serde_json::Value) -> SessionEvent {
        SessionEvent {
            seq: 1,
            id: SessionEventId::new(),
            session_id: SessionId::new(),
            run_id: None,
            turn_id: None,
            event_type,
            payload_json,
            transient: false,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn queued_turns_do_not_publish_working_activity() {
        let queued = derive_summary_activity(&test_event(SessionEventType::TurnQueued, json!({})))
            .expect("queued turns should publish summary activity");
        assert!(!queued.is_working);
        assert_eq!(queued.last_turn_status, Some(SessionTurnStatus::Queued));

        let running =
            derive_summary_activity(&test_event(SessionEventType::TurnStarted, json!({})))
                .expect("running turns should publish summary activity");
        assert!(running.is_working);
        assert_eq!(running.last_turn_status, Some(SessionTurnStatus::Running));
    }

    #[test]
    fn turn_finished_summary_activity_uses_embedded_status() {
        let interrupted = derive_summary_activity(&test_event(
            SessionEventType::TurnFinished,
            json!({"status": "interrupted"}),
        ))
        .expect("interrupt finish should publish summary activity");
        assert_eq!(
            interrupted.last_turn_status,
            Some(SessionTurnStatus::Interrupted)
        );

        let failed = derive_summary_activity(&test_event(
            SessionEventType::TurnFinished,
            json!({"status": "failed"}),
        ))
        .expect("failed finish should publish summary activity");
        assert_eq!(failed.last_turn_status, Some(SessionTurnStatus::Failed));
    }

    #[test]
    fn raw_terminal_events_do_not_publish_terminal_summary_activity() {
        assert!(derive_summary_activity(&test_event(SessionEventType::Done, json!({}))).is_none());
        assert!(derive_summary_activity(&test_event(SessionEventType::Error, json!({}))).is_none());
        assert!(
            derive_summary_activity(&test_event(SessionEventType::TurnInterrupted, json!({})))
                .is_none()
        );
    }

    #[test]
    fn emitted_session_summary_deltas_always_include_monotonic_versions() {
        let session = test_session();
        let now = Utc::now();

        let message_delta = build_session_summary_delta(
            &session,
            None,
            Some(now),
            Some("preview".to_string()),
            22,
            22,
            22,
        )
        .expect("message preview should emit a summary delta");
        assert_eq!(message_delta.last_event_seq, Some(22));
        assert_eq!(message_delta.projection_rev, Some(22));
        assert_eq!(message_delta.state_rev, Some(22));
    }

    #[test]
    fn empty_session_summary_delta_is_not_emitted() {
        let session = test_session();
        assert!(
            build_session_summary_delta(&session, None, None, None, 5, 5, 5).is_none(),
            "empty updates should not publish summary deltas"
        );
    }

    #[test]
    fn activity_only_session_summary_delta_is_emitted() {
        let session = test_session();
        let delta = build_session_summary_delta(
            &session,
            Some(SessionActivityState {
                is_working: true,
                last_turn_status: Some(SessionTurnStatus::Running),
            }),
            None,
            None,
            5,
            5,
            5,
        )
        .expect("activity updates should emit a summary delta");
        assert!(delta.activity.expect("activity delta").is_working);
        assert_eq!(delta.last_event_seq, Some(5));
    }

    #[tokio::test]
    async fn stream_only_projection_rev_skips_lookup() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_lookup = Arc::clone(&calls);
        let projection_rev =
            resolve_projection_rev_for_stream_delta(true, 41, 23, move || async move {
                calls_for_lookup.fetch_add(1, Ordering::SeqCst);
                Some(99)
            })
            .await;

        assert_eq!(projection_rev, 23);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn non_stream_only_projection_rev_uses_lookup_when_available() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_lookup = Arc::clone(&calls);
        let projection_rev =
            resolve_projection_rev_for_stream_delta(false, 17, 7, move || async move {
                calls_for_lookup.fetch_add(1, Ordering::SeqCst);
                Some(23)
            })
            .await;

        assert_eq!(projection_rev, 23);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn detects_session_gap_notice() {
        let event = test_event(
            SessionEventType::Notice,
            json!({
                "kind": "session_gap",
                "reason": "data_plane_overflow",
            }),
        );
        assert!(is_session_gap_notice(&event));
    }

    #[test]
    fn ignores_non_gap_notice() {
        let event = test_event(
            SessionEventType::Notice,
            json!({
                "kind": "context.compacted",
            }),
        );
        assert!(!is_session_gap_notice(&event));
    }

    #[test]
    fn ignores_non_notice_events() {
        let event = test_event(
            SessionEventType::ToolResult,
            json!({
                "kind": "session_gap",
            }),
        );
        assert!(!is_session_gap_notice(&event));
    }
}
