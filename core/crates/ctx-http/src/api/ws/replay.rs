use super::*;

pub(super) use crate::daemon::workspaces::stream::{
    load_worktree_vcs_snapshots_for_sessions, merge_worktree_vcs_snapshots,
    primary_session_id_for_active_task, primary_session_ids_for_active_task_summary,
    refresh_worktree_vcs_for_sessions, replay_session_events,
    resolve_workspace_active_snapshot_subscriptions, resolve_worktree_vcs_open_session_ids,
    resolve_worktree_vcs_publish_worktree_ids, resolve_worktree_vcs_summary_session_ids,
    spawn_worktree_vcs_refresh_for_sessions, sync_active_worktrees, ReplayOutcome,
};

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
        WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => {
            WorkspaceActiveSnapshotStreamMessage::Event {
                rev: stream_rev,
                event,
            }
        }
        WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
            snapshot_rev,
            deltas,
            ..
        } => WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
            rev: stream_rev,
            snapshot_rev,
            deltas,
        },
        WorkspaceActiveSnapshotStreamMessage::ResetRequired { latest_rev } => {
            WorkspaceActiveSnapshotStreamMessage::ResetRequired { latest_rev }
        }
    }
}

pub(super) async fn queue_reset_required(
    pending: &StreamQueue<WorkspaceActiveSnapshotStreamMessage>,
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<(), ()> {
    let (snapshot_rev, _) =
        super::super::tasks::load_workspace_active_snapshot_state(state, workspace_id).await;
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
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    worktree_vcs_summary_session_ids: &[SessionId],
    worktree_vcs_open_session_ids: &[SessionId],
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
    crate::merge_queue::activate_workspace_merge_queue(state, workspace_id).await;
    let mut active_snapshot = state
        .workspaces
        .workspace_active_snapshot
        .active_snapshot(workspace_id, i64::MAX)
        .await;
    let publish_worktree_ids = resolve_worktree_vcs_publish_worktree_ids(
        state,
        worktree_vcs_summary_session_ids,
        worktree_vcs_open_session_ids,
    )
    .await;
    active_snapshot
        .worktree_vcs_snapshots
        .retain(|snapshot| publish_worktree_ids.contains(&snapshot.worktree_id));
    let mut extra_worktree_vcs_snapshots =
        load_worktree_vcs_snapshots_for_sessions(state, worktree_vcs_summary_session_ids).await;
    let open_worktree_vcs_snapshots =
        load_worktree_vcs_snapshots_for_sessions(state, worktree_vcs_open_session_ids).await;
    extra_worktree_vcs_snapshots =
        merge_worktree_vcs_snapshots(extra_worktree_vcs_snapshots, open_worktree_vcs_snapshots);
    active_snapshot.worktree_vcs_snapshots = merge_worktree_vcs_snapshots(
        active_snapshot.worktree_vcs_snapshots,
        extra_worktree_vcs_snapshots,
    );
    let active_heads = state
        .workspaces
        .workspace_active_snapshot
        .active_heads(workspace_id)
        .await;
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

pub(super) enum WorktreeVcsSeedMode {
    IncludedInSnapshot,
    EmitCachedEvents,
}

pub(super) async fn seed_worktree_vcs_for_subscribe(
    pending: &StreamQueue<WorkspaceActiveSnapshotStreamMessage>,
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_ids: &[SessionId],
    seed_mode: WorktreeVcsSeedMode,
) -> Result<(), ()> {
    if matches!(seed_mode, WorktreeVcsSeedMode::IncludedInSnapshot) {
        return Ok(());
    }
    let snapshots = load_worktree_vcs_snapshots_for_sessions(state, session_ids).await;
    if snapshots.is_empty() {
        return Ok(());
    }
    let (snapshot_rev, _) =
        super::super::tasks::load_workspace_active_snapshot_state(state, workspace_id).await;
    for snapshot in snapshots {
        push_stream_message(
            pending,
            workspace_id,
            None,
            "worktree_vcs_seed",
            WorkspaceActiveSnapshotStreamMessage::Event {
                rev: 0,
                event: Box::new(WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot {
                    workspace_id,
                    snapshot_rev,
                    snapshot: Box::new(snapshot),
                }),
            },
        )
        .await?;
    }
    Ok(())
}
