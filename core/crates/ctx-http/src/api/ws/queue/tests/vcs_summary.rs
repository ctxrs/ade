use super::*;

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
