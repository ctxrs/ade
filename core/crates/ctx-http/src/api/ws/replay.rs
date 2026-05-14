use super::*;

pub(super) fn with_stream_rev(
    message: WorkspaceActiveSnapshotStreamMessage,
    stream_rev: i64,
) -> WorkspaceActiveSnapshotStreamMessage {
    match message {
        WorkspaceActiveSnapshotStreamMessage::Snapshot {
            active_snapshot,
            active_heads,
            ..
        } => WorkspaceActiveSnapshotStreamMessage::Snapshot {
            rev: stream_rev,
            active_snapshot,
            active_heads,
        },
        WorkspaceActiveSnapshotStreamMessage::Event {
            event,
            stream_source,
            ..
        } => WorkspaceActiveSnapshotStreamMessage::Event {
            rev: stream_rev,
            event,
            stream_source,
        },
        WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
            snapshot_rev,
            deltas,
            stream_source,
            ..
        } => WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
            rev: stream_rev,
            snapshot_rev,
            deltas,
            stream_source,
        },
        WorkspaceActiveSnapshotStreamMessage::ResetRequired { latest_rev } => {
            WorkspaceActiveSnapshotStreamMessage::ResetRequired { latest_rev }
        }
    }
}

pub(super) async fn queue_reset_required(
    pending: &StreamQueue<WorkspaceActiveSnapshotStreamMessage>,
    state: &WorkspaceStreamHandle,
    workspace_id: WorkspaceId,
) -> Result<(), ()> {
    let (snapshot_rev, _) = state
        .load_workspace_active_snapshot_state(workspace_id)
        .await;
    if crate::fault_injection::maybe_fail("ctx_http.send_workspace_active_reset").is_err() {
        return Err(());
    }
    push_stream_message(
        pending,
        workspace_id,
        None,
        "reset_required",
        WorkspaceActiveSnapshotStreamMessage::ResetRequired {
            latest_rev: snapshot_rev,
        },
    )
    .await
}

pub(super) async fn queue_snapshot_payload(
    pending: &StreamQueue<WorkspaceActiveSnapshotStreamMessage>,
    state: &WorkspaceStreamHandle,
    workspace_id: WorkspaceId,
) -> Result<(), ()> {
    let build_start = Instant::now();
    state
        .ensure_workspace_active_snapshot_hydrated(workspace_id)
        .await
        .map_err(|err| {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                "workspace snapshot hydration failed before snapshot payload: {err:?}"
            );
        })?;
    state.activate_workspace_merge_queue(workspace_id).await;
    let active_snapshot = state.workspace_active_snapshot(workspace_id).await;
    let active_heads = state.workspace_active_heads(workspace_id).await;
    let snapshot_rev = active_snapshot.snapshot_rev;
    let task_count = active_snapshot.active.tasks.len();
    let head_count = active_heads.heads.len();
    let build_ms = build_start.elapsed().as_millis();
    if crate::fault_injection::maybe_fail("ctx_http.send_workspace_active_snapshot").is_err() {
        return Err(());
    }
    push_stream_message(
        pending,
        workspace_id,
        None,
        "snapshot",
        WorkspaceActiveSnapshotStreamMessage::Snapshot {
            rev: 0,
            active_snapshot,
            active_heads: Some(active_heads),
        },
    )
    .await?;
    tracing::info!(
        target: "ctx_http.ws_active_snapshot",
        workspace_id = %workspace_id.0,
        snapshot_rev,
        active_tasks = task_count,
        active_heads = head_count,
        snapshot_build_ms = build_ms,
        "workspace snapshot queued",
    );
    Ok(())
}
