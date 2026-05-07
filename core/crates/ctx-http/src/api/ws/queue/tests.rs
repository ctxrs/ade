use super::partials::merge_partial_fragment;
use super::*;
use chrono::Utc;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use ctx_core::ids::*;
use ctx_core::models::*;
use ctx_workspace_active_snapshot::SessionReplayCursor;

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

fn cursor_delta(session_id: SessionId, last_event_seq: i64) -> SessionHeadDelta {
    cursor_delta_with_projection(session_id, last_event_seq, 7)
}

fn cursor_delta_with_projection(
    session_id: SessionId,
    last_event_seq: i64,
    projection_rev: i64,
) -> SessionHeadDelta {
    let mut delta = partial_delta(session_id);
    delta.last_event_seq = last_event_seq;
    delta.projection_rev = projection_rev;
    if let Some(event) = delta.event.as_mut() {
        event.seq = last_event_seq;
        event.event_type = SessionEventType::Notice;
        event.transient = false;
        event.payload_json = json!({ "seq": last_event_seq, "projectionRev": projection_rev });
    }
    delta
}

fn session_summary_delta_event(
    workspace_id: WorkspaceId,
    session_id: SessionId,
    last_event_seq: i64,
) -> WorkspaceActiveSnapshotEvent {
    session_summary_delta_event_with_projection(workspace_id, session_id, last_event_seq, 7)
}

fn session_summary_delta_event_with_projection(
    workspace_id: WorkspaceId,
    session_id: SessionId,
    last_event_seq: i64,
    projection_rev: i64,
) -> WorkspaceActiveSnapshotEvent {
    WorkspaceActiveSnapshotEvent::SessionSummaryDelta {
        workspace_id,
        snapshot_rev: last_event_seq,
        delta: Box::new(SessionSummaryDelta {
            session_id,
            task_id: TaskId::new(),
            activity: None,
            last_message_at: None,
            last_message_preview: None,
            last_event_seq: Some(last_event_seq),
            projection_rev: Some(projection_rev),
            state_rev: None,
            emitted_at_ms: None,
        }),
    }
}

fn worktree_vcs_event(
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    rev: i64,
) -> WorkspaceActiveSnapshotEvent {
    WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot {
        workspace_id,
        snapshot_rev: rev,
        snapshot: Box::new(WorktreeVcsSnapshot {
            worktree_id,
            rev,
            emitted_at_ms: rev,
            base_commit_sha: "base".to_string(),
            head_commit_sha: "head".to_string(),
            target_branch: Some("origin/main".to_string()),
            target_branch_commit_sha: Some("target".to_string()),
            base_resolution: WorktreeVcsBaseResolution::default(),
            compute_state: WorktreeVcsComputeState::Ready,
            summary: WorktreeVcsSummary::default(),
            git_status: WorktreeVcsGitStatusSummary::default(),
            touched_files: WorktreeVcsTouchedFiles::default(),
            touched_files_state: WorktreeVcsTouchedFilesState::Ready,
            freshness: WorktreeVcsFreshness::Fresh,
            available: true,
            unavailable_reason: None,
            schema_version: 1,
        }),
    }
}

#[tokio::test]
async fn head_buffer_drops_session_deltas_at_or_before_resume_cursor() {
    let buffer = HeadBatchBuffer::new();
    let session_id = SessionId::new();
    let other_session_id = SessionId::new();

    buffer
        .push(1, cursor_delta(session_id, 2))
        .await
        .expect("old delta should enqueue");
    buffer
        .push(2, cursor_delta_with_projection(session_id, 3, 7))
        .await
        .expect("cursor delta should enqueue");
    buffer
        .push(3, cursor_delta_with_projection(session_id, 3, 8))
        .await
        .expect("same-seq projection delta should enqueue");
    buffer
        .push(4, cursor_delta_with_projection(session_id, 5, 0))
        .await
        .expect("newer delta should enqueue");
    buffer
        .push(5, cursor_delta(other_session_id, 1))
        .await
        .expect("other session delta should enqueue");

    buffer
        .drop_session_deltas_at_or_before(
            session_id,
            SessionReplayCursor {
                last_event_seq: 3,
                projection_rev: 7,
            },
        )
        .await;

    let (_, deltas) = buffer.take().await;
    assert_eq!(deltas.len(), 3);
    assert!(deltas.iter().any(|delta| delta.session_id == session_id
        && delta.last_event_seq == 3
        && delta.projection_rev == 8));
    assert!(deltas.iter().any(|delta| delta.session_id == session_id
        && delta.last_event_seq == 5
        && delta.projection_rev == 0));
    assert!(deltas
        .iter()
        .any(|delta| delta.session_id == other_session_id));
    assert!(!deltas.iter().any(|delta| delta.session_id == session_id
        && SessionReplayCursor::from_delta(delta)
            <= SessionReplayCursor {
                last_event_seq: 3,
                projection_rev: 7,
            }));
}

#[tokio::test]
async fn summary_buffer_drops_session_events_at_or_before_resume_cursor() {
    let buffer = SummaryBatchBuffer::new(8);
    let workspace_id = WorkspaceId::new();
    let session_id = SessionId::new();
    let other_session_id = SessionId::new();
    let worktree_id = WorktreeId::new();

    buffer
        .push(session_summary_delta_event(workspace_id, session_id, 3))
        .await
        .expect("summary delta should enqueue");
    buffer
        .push(session_summary_delta_event_with_projection(
            workspace_id,
            session_id,
            3,
            8,
        ))
        .await
        .expect("newer projection summary delta should replace");
    buffer
        .push(session_summary_delta_event(
            workspace_id,
            other_session_id,
            1,
        ))
        .await
        .expect("other summary delta should enqueue");
    buffer
        .push(worktree_vcs_event(workspace_id, worktree_id, 2))
        .await
        .expect("VCS delta should enqueue");

    buffer
        .drop_session_events_at_or_before(
            session_id,
            SessionReplayCursor {
                last_event_seq: 3,
                projection_rev: 7,
            },
        )
        .await;

    let (events, vcs_coalesced_count) = buffer.take().await;
    assert_eq!(events.len(), 3);
    assert_eq!(vcs_coalesced_count, 0);
    assert!(events.iter().any(|event| matches!(
        event,
        WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. }
            if delta.session_id == session_id && delta.projection_rev == Some(8)
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. }
            if delta.session_id == other_session_id
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot { snapshot, .. }
            if snapshot.worktree_id == worktree_id
    )));
}

#[tokio::test]
async fn summary_buffer_removes_session_event_at_resume_cursor() {
    let buffer = SummaryBatchBuffer::new(8);
    let workspace_id = WorkspaceId::new();
    let session_id = SessionId::new();
    let worktree_id = WorktreeId::new();

    buffer
        .push(session_summary_delta_event_with_projection(
            workspace_id,
            session_id,
            3,
            7,
        ))
        .await
        .expect("summary delta should enqueue");
    buffer
        .push(worktree_vcs_event(workspace_id, worktree_id, 2))
        .await
        .expect("VCS delta should enqueue");

    buffer
        .drop_session_events_at_or_before(
            session_id,
            SessionReplayCursor {
                last_event_seq: 3,
                projection_rev: 7,
            },
        )
        .await;

    let (events, vcs_coalesced_count) = buffer.take().await;
    assert_eq!(events.len(), 1);
    assert_eq!(vcs_coalesced_count, 0);
    assert!(events.iter().any(|event| matches!(
        event,
        WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot { snapshot, .. }
            if snapshot.worktree_id == worktree_id
    )));
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
async fn summary_buffer_coalesces_worktree_vcs_snapshots_latest_wins() {
    let buffer = SummaryBatchBuffer::new(8);
    let workspace_id = WorkspaceId::new();
    let worktree_id = WorktreeId::new();
    let other_worktree_id = WorktreeId::new();

    buffer
        .push(worktree_vcs_event(workspace_id, worktree_id, 1))
        .await
        .expect("first VCS event should enqueue");
    buffer
        .push(worktree_vcs_event(workspace_id, worktree_id, 2))
        .await
        .expect("same worktree VCS event should replace pending event");
    buffer
        .push(worktree_vcs_event(workspace_id, other_worktree_id, 3))
        .await
        .expect("different worktree VCS event should enqueue");

    let (events, vcs_coalesced_count) = buffer.take().await;
    assert_eq!(events.len(), 2);
    assert_eq!(vcs_coalesced_count, 1);
    assert!(events.iter().any(|event| matches!(
        event,
        WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot { snapshot, .. }
            if snapshot.worktree_id == worktree_id && snapshot.rev == 2
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot { snapshot, .. }
            if snapshot.worktree_id == other_worktree_id && snapshot.rev == 3
    )));
}

#[tokio::test]
async fn next_workspace_stream_item_keeps_vcs_snapshots_behind_foreground_heads() {
    let priority_control = StreamQueue::new(8, Duration::from_secs(1));
    let control = StreamQueue::new(8, Duration::from_secs(1));
    let foreground_head_buffer = HeadBatchBuffer::new();
    let background_head_buffer = HeadBatchBuffer::new();
    let summary_buffer = SummaryBatchBuffer::new(8);
    let workspace_id = WorkspaceId::new();
    let foreground_session_id = SessionId::new();
    let worktree_id = WorktreeId::new();

    summary_buffer
        .push(worktree_vcs_event(workspace_id, worktree_id, 1))
        .await
        .expect("VCS event should enqueue in summary lane");
    foreground_head_buffer
        .push(2, partial_delta(foreground_session_id))
        .await
        .expect("foreground delta should enqueue");

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
        panic!("expected foreground head before VCS summary lane");
    };
    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].session_id, foreground_session_id);

    let Some(NextWorkspaceStreamItem::SummaryBatch {
        events,
        vcs_coalesced_count,
    }) = take_next_workspace_stream_item(
        &priority_control,
        &control,
        &foreground_head_buffer,
        &background_head_buffer,
        &summary_buffer,
        false,
    )
    .await
    else {
        panic!("expected VCS event after foreground head");
    };
    assert_eq!(events.len(), 1);
    assert_eq!(vcs_coalesced_count, 0);
    assert!(matches!(
        &events[0],
        WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot { snapshot, .. }
            if snapshot.worktree_id == worktree_id
    ));
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
