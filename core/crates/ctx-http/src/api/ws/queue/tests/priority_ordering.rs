use super::*;

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
