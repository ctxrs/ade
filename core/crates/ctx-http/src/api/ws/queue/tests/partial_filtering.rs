use super::*;

#[test]
fn filter_partial_delta_drops_non_primary_partial_only_delta() {
    let session_id = SessionId::new();
    let delta = partial_delta(session_id);
    let active_task_sessions = HashMap::new();

    assert!(filter_partial_delta_for_active_tasks(delta, &active_task_sessions, None).is_none());
}

#[test]
fn filter_partial_delta_drops_primary_partial_delta_when_not_foreground() {
    let session_id = SessionId::new();
    let delta = partial_delta(session_id);
    let mut active_task_sessions = HashMap::new();
    active_task_sessions.insert(TaskId::new(), session_id);

    assert!(filter_partial_delta_for_active_tasks(delta, &active_task_sessions, None).is_none());
}

#[test]
fn filter_partial_delta_keeps_foreground_secondary_session_partial_delta() {
    let session_id = SessionId::new();
    let delta = partial_delta(session_id);
    let active_task_sessions = HashMap::new();
    let mut foreground_session_ids = HashSet::new();
    foreground_session_ids.insert(session_id);

    let filtered = filter_partial_delta_for_active_tasks(
        delta,
        &active_task_sessions,
        Some(&foreground_session_ids),
    )
    .expect("foreground session partial delta should be preserved");
    assert!(filtered.event.is_some());
}

#[test]
fn should_stream_head_delta_keeps_explicit_session() {
    let session_id = SessionId::new();
    let active_task_sessions = HashMap::new();
    let mut explicit_sessions = HashSet::new();
    explicit_sessions.insert(session_id);

    assert!(should_stream_head_delta(
        &active_task_sessions,
        &explicit_sessions,
        None,
        session_id,
    ));
}

#[test]
fn should_stream_head_delta_keeps_foreground_session() {
    let session_id = SessionId::new();
    let active_task_sessions = HashMap::new();
    let explicit_sessions = HashSet::new();
    let mut foreground_session_ids = HashSet::new();
    foreground_session_ids.insert(session_id);

    assert!(should_stream_head_delta(
        &active_task_sessions,
        &explicit_sessions,
        Some(&foreground_session_ids),
        session_id,
    ));
}

#[test]
fn should_stream_head_delta_drops_non_active_non_explicit_background_session() {
    let session_id = SessionId::new();
    let active_task_sessions = HashMap::new();
    let explicit_sessions = HashSet::new();

    assert!(!should_stream_head_delta(
        &active_task_sessions,
        &explicit_sessions,
        None,
        session_id,
    ));
}

#[test]
fn filter_partial_delta_preserves_non_event_payloads() {
    let session_id = SessionId::new();
    let mut delta = partial_delta(session_id);
    let now = Utc::now();
    delta.tool_summaries.push(SessionTurnToolSummary {
        session_id,
        tool_call_id: "call-1".to_string(),
        turn_id: TurnId::new(),
        tool_kind: Some("function".to_string()),
        provider_tool_name: Some("shell".to_string()),
        title: Some("Shell".to_string()),
        subtitle: None,
        status: Some("running".to_string()),
        input_preview: None,
        output_preview: None,
        order_seq: 1,
        first_event_seq: None,
        input_truncated: None,
        input_original_bytes: None,
        output_truncated: None,
        output_original_bytes: None,
        created_at: now,
        updated_at: now,
    });
    let active_task_sessions = HashMap::new();

    let filtered = filter_partial_delta_for_active_tasks(delta, &active_task_sessions, None)
        .expect("non-event payload should keep delta sendable");
    assert!(filtered.event.is_none());
    assert_eq!(filtered.tool_summaries.len(), 1);
}

#[test]
fn merge_partial_fragment_appends_non_overlapping_text() {
    assert_eq!(
        merge_partial_fragment("partial", " fragment"),
        "partial fragment"
    );
}
