use super::*;

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
async fn head_buffer_take_chunk_keeps_remaining_deltas() {
    let buffer = HeadBatchBuffer::new();
    let session_id = SessionId::new();

    buffer
        .push(10, cursor_delta(session_id, 1))
        .await
        .expect("first delta should enqueue");
    buffer
        .push(11, cursor_delta(session_id, 2))
        .await
        .expect("second delta should enqueue");
    buffer
        .push(12, cursor_delta(session_id, 3))
        .await
        .expect("third delta should enqueue");

    let first = buffer.take_chunk_with_meta(2).await;
    assert_eq!(first.snapshot_rev, 12);
    assert_eq!(first.deltas.len(), 2);
    assert_eq!(first.deltas[0].last_event_seq, 1);
    assert_eq!(first.deltas[1].last_event_seq, 2);

    let second = buffer.take_chunk_with_meta(2).await;
    assert_eq!(second.snapshot_rev, 12);
    assert_eq!(second.deltas.len(), 1);
    assert_eq!(second.deltas[0].last_event_seq, 3);

    let empty = buffer.take_chunk_with_meta(2).await;
    assert!(empty.deltas.is_empty());
}

#[tokio::test]
async fn head_buffer_zero_chunk_does_not_drop_pending_deltas() {
    let buffer = HeadBatchBuffer::new();
    let session_id = SessionId::new();

    buffer
        .push(10, cursor_delta(session_id, 1))
        .await
        .expect("delta should enqueue");

    let skipped = buffer.take_chunk_with_meta(0).await;
    assert!(skipped.deltas.is_empty());

    let drained = buffer.take_chunk_with_meta(1).await;
    assert_eq!(drained.deltas.len(), 1);
    assert_eq!(drained.deltas[0].last_event_seq, 1);
}

#[tokio::test]
async fn summary_buffer_drops_session_events_at_or_before_resume_cursor() {
    let buffer = SummaryBatchBuffer::new(8);
    let workspace_id = WorkspaceId::new();
    let session_id = SessionId::new();
    let other_session_id = SessionId::new();

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
        .drop_session_events_at_or_before(
            session_id,
            SessionReplayCursor {
                last_event_seq: 3,
                projection_rev: 7,
            },
        )
        .await;

    let events = buffer.take().await;
    assert_eq!(events.len(), 2);
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
}

#[tokio::test]
async fn summary_buffer_removes_session_event_at_resume_cursor() {
    let buffer = SummaryBatchBuffer::new(8);
    let workspace_id = WorkspaceId::new();
    let session_id = SessionId::new();

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
        .drop_session_events_at_or_before(
            session_id,
            SessionReplayCursor {
                last_event_seq: 3,
                projection_rev: 7,
            },
        )
        .await;

    let events = buffer.take().await;
    assert!(events.is_empty());
}
