use super::partials::merge_partial_fragment;
use super::*;
use chrono::Utc;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use ctx_core::ids::*;
use ctx_core::models::*;

fn partial_delta(session_id: SessionId) -> SessionHeadDelta {
    SessionHeadDelta {
        session_id,
        last_event_seq: 5,
        projection_rev: 7,
        state_rev: 0,
        emitted_at_ms: None,
        session: None,
        activity: None,
        event: Some(SessionEvent {
            seq: -1,
            id: SessionEventId::new(),
            session_id,
            run_id: None,
            turn_id: Some(TurnId::new()),
            event_type: SessionEventType::AssistantChunk,
            payload_json: json!({ "content_fragment": "partial" }),
            transient: true,
            created_at: Utc::now(),
        }),
        turn: None,
        message: None,
        tool_summaries: Vec::new(),
    }
}

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

#[tokio::test]
async fn next_workspace_stream_item_prioritizes_foreground_lane() {
    let priority_control = StreamQueue::new(8, Duration::from_secs(1));
    let control = StreamQueue::new(8, Duration::from_secs(1));
    let foreground_head_buffer = HeadBatchBuffer::new();
    let background_head_buffer = HeadBatchBuffer::new();
    let summary_buffer = SummaryBatchBuffer::new(8);
    let workspace_id = WorkspaceId::new();
    let foreground_session_id = SessionId::new();
    let background_session_id = SessionId::new();

    push_stream_message(
        &control,
        workspace_id,
        Some(background_session_id),
        "test_control",
        WorkspaceActiveSnapshotStreamMessage::Event {
            rev: 0,
            event: Box::new(WorkspaceActiveSnapshotEvent::SessionGap {
                workspace_id,
                snapshot_rev: 1,
                session_id: background_session_id,
                after_seq: 10,
                reason: Some("background".to_string()),
            }),
        },
    )
    .await
    .expect("control event should enqueue");
    foreground_head_buffer
        .push(2, partial_delta(foreground_session_id))
        .await
        .expect("foreground delta should enqueue");
    background_head_buffer
        .push(3, partial_delta(background_session_id))
        .await
        .expect("background delta should enqueue");
    push_stream_message(
        &priority_control,
        workspace_id,
        Some(foreground_session_id),
        "test_priority",
        WorkspaceActiveSnapshotStreamMessage::Event {
            rev: 0,
            event: Box::new(WorkspaceActiveSnapshotEvent::SessionGap {
                workspace_id,
                snapshot_rev: 4,
                session_id: foreground_session_id,
                after_seq: 11,
                reason: Some("foreground".to_string()),
            }),
        },
    )
    .await
    .expect("priority control event should enqueue");

    let Some(NextWorkspaceStreamItem::Control(entry)) = take_next_workspace_stream_item(
        &priority_control,
        &control,
        &foreground_head_buffer,
        &background_head_buffer,
        &summary_buffer,
        false,
    )
    .await
    else {
        panic!("expected priority control event first");
    };
    let (_, message) = entry.into_parts();
    assert!(matches!(
        message,
        WorkspaceActiveSnapshotStreamMessage::Event { event, .. }
            if matches!(
                event.as_ref(),
                WorkspaceActiveSnapshotEvent::SessionGap { session_id, .. }
                    if *session_id == foreground_session_id
            )
    ));

    let Some(NextWorkspaceStreamItem::HeadsBatch { deltas, .. }) = take_next_workspace_stream_item(
        &priority_control,
        &control,
        &foreground_head_buffer,
        &background_head_buffer,
        &summary_buffer,
        false,
    )
    .await
    else {
        panic!("expected foreground heads batch second");
    };
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].session_id, foreground_session_id);

    let Some(NextWorkspaceStreamItem::Control(entry)) = take_next_workspace_stream_item(
        &priority_control,
        &control,
        &foreground_head_buffer,
        &background_head_buffer,
        &summary_buffer,
        false,
    )
    .await
    else {
        panic!("expected background control event third");
    };
    let (_, message) = entry.into_parts();
    assert!(matches!(
        message,
        WorkspaceActiveSnapshotStreamMessage::Event { event, .. }
            if matches!(
                event.as_ref(),
                WorkspaceActiveSnapshotEvent::SessionGap { session_id, .. }
                    if *session_id == background_session_id
            )
    ));

    let Some(NextWorkspaceStreamItem::HeadsBatch { deltas, .. }) = take_next_workspace_stream_item(
        &priority_control,
        &control,
        &foreground_head_buffer,
        &background_head_buffer,
        &summary_buffer,
        false,
    )
    .await
    else {
        panic!("expected background heads batch last");
    };
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].session_id, background_session_id);
}

#[tokio::test]
async fn priority_control_event_detection_only_matches_foreground_session() {
    let workspace_id = WorkspaceId::new();
    let foreground_session_id = SessionId::new();
    let background_session_id = SessionId::new();
    let mut foreground_session_ids = HashSet::new();
    foreground_session_ids.insert(foreground_session_id);
    let foreground_gap = WorkspaceActiveSnapshotEvent::SessionGap {
        workspace_id,
        snapshot_rev: 1,
        session_id: foreground_session_id,
        after_seq: 1,
        reason: Some("foreground".to_string()),
    };
    let background_gap = WorkspaceActiveSnapshotEvent::SessionGap {
        workspace_id,
        snapshot_rev: 1,
        session_id: background_session_id,
        after_seq: 1,
        reason: Some("background".to_string()),
    };

    assert!(is_priority_control_event(
        &foreground_gap,
        Some(&foreground_session_ids),
    ));
    assert!(!is_priority_control_event(
        &background_gap,
        Some(&foreground_session_ids),
    ));
}

#[tokio::test]
async fn hydrating_keeps_snapshot_control_ahead_of_priority_lane() {
    let priority_control = StreamQueue::new(8, Duration::from_secs(1));
    let control = StreamQueue::new(8, Duration::from_secs(1));
    let foreground_head_buffer = HeadBatchBuffer::new();
    let background_head_buffer = HeadBatchBuffer::new();
    let summary_buffer = SummaryBatchBuffer::new(8);
    let workspace_id = WorkspaceId::new();
    let foreground_session_id = SessionId::new();

    push_stream_message(
        &control,
        workspace_id,
        None,
        "test_snapshot",
        WorkspaceActiveSnapshotStreamMessage::Snapshot {
            rev: 0,
            active_snapshot: WorkspaceActiveSnapshot {
                workspace_id,
                snapshot_rev: 1,
                archived_rev: 0,
                active: WorkspaceActivePage {
                    tasks: Vec::new(),
                    total_count: 0,
                },
                worktree_vcs_snapshots: Vec::new(),
            },
            active_heads: None,
        },
    )
    .await
    .expect("snapshot should enqueue");
    push_stream_message(
        &priority_control,
        workspace_id,
        Some(foreground_session_id),
        "test_priority",
        WorkspaceActiveSnapshotStreamMessage::Event {
            rev: 0,
            event: Box::new(WorkspaceActiveSnapshotEvent::SessionGap {
                workspace_id,
                snapshot_rev: 2,
                session_id: foreground_session_id,
                after_seq: 1,
                reason: Some("foreground".to_string()),
            }),
        },
    )
    .await
    .expect("priority event should enqueue");

    let Some(NextWorkspaceStreamItem::Control(entry)) = take_next_workspace_stream_item(
        &priority_control,
        &control,
        &foreground_head_buffer,
        &background_head_buffer,
        &summary_buffer,
        true,
    )
    .await
    else {
        panic!("expected snapshot control while hydrating");
    };
    let (_, message) = entry.into_parts();
    assert!(matches!(
        message,
        WorkspaceActiveSnapshotStreamMessage::Snapshot { .. }
    ));

    let Some(NextWorkspaceStreamItem::Control(entry)) = take_next_workspace_stream_item(
        &priority_control,
        &control,
        &foreground_head_buffer,
        &background_head_buffer,
        &summary_buffer,
        false,
    )
    .await
    else {
        panic!("expected priority event after hydration");
    };
    let (_, message) = entry.into_parts();
    assert!(matches!(
        message,
        WorkspaceActiveSnapshotStreamMessage::Event { event, .. }
            if matches!(
                event.as_ref(),
                WorkspaceActiveSnapshotEvent::SessionGap { session_id, .. }
                    if *session_id == foreground_session_id
            )
    ));
}
