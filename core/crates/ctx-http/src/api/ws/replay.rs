use super::*;

pub(super) enum ReplayOutcome {
    Replay { last_sent: i64 },
    ResetRequired,
}

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
) -> Result<(), ()> {
    let build_start = Instant::now();
    state
        .ensure_workspace_active_snapshot_hydrated(workspace_id)
        .await;
    let mut active_snapshot = state
        .workspaces
        .workspace_active_snapshot
        .active_snapshot(workspace_id, i64::MAX)
        .await;
    let mut active_worktree_ids = HashSet::new();
    for task in &active_snapshot.active.tasks {
        active_worktree_ids.insert(task.primary_session.session.worktree_id);
        for summary in &task.sessions {
            active_worktree_ids.insert(summary.session.worktree_id);
        }
    }
    let mut worktree_vcs_snapshots = Vec::new();
    for worktree_id in active_worktree_ids {
        if let Some(snapshot) = state.get_worktree_vcs_snapshot(worktree_id).await {
            worktree_vcs_snapshots.push(snapshot);
        }
    }
    active_snapshot.worktree_vcs_snapshots = worktree_vcs_snapshots;
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

pub(super) async fn replay_session_events<F, Fut>(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    after_seq: i64,
    list_failpoint: &'static str,
    send_failpoint: Option<&'static str>,
    mut emit: F,
) -> Result<ReplayOutcome, ()>
where
    F: FnMut(WorkspaceActiveSnapshotStreamMessage) -> Fut,
    Fut: std::future::Future<Output = Result<(), ()>>,
{
    let (snapshot_rev, _) =
        super::super::tasks::load_workspace_active_snapshot_state(state, workspace_id).await;
    if crate::fault_injection::maybe_fail(list_failpoint).is_err() {
        return Ok(ReplayOutcome::ResetRequired);
    }
    let replay = state
        .workspaces
        .workspace_active_snapshot
        .replay_session_stream(
            workspace_id,
            session_id,
            after_seq,
            SESSION_REPLAY_MAX_EVENTS,
        )
        .await;
    match replay {
        WorkspaceSessionReplay::Replay { items, last_sent } => {
            for item in items {
                if matches!(item, WorkspaceSessionReplayItem::Delta(_)) {
                    if let Some(label) = send_failpoint {
                        crate::fault_injection::maybe_fail(label).map_err(|_| ())?;
                    }
                }
                let event = match item {
                    WorkspaceSessionReplayItem::Delta(delta) => {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                            workspace_id,
                            snapshot_rev,
                            delta: Box::new(delta),
                        }
                    }
                    WorkspaceSessionReplayItem::Gap {
                        session_id,
                        after_seq,
                        reason,
                    } => WorkspaceActiveSnapshotEvent::SessionGap {
                        workspace_id,
                        snapshot_rev,
                        session_id,
                        after_seq,
                        reason,
                    },
                    WorkspaceSessionReplayItem::Seed(head) => {
                        WorkspaceActiveSnapshotEvent::SessionHeadSeed {
                            workspace_id,
                            snapshot_rev,
                            head: Box::new(head),
                        }
                    }
                };
                emit(WorkspaceActiveSnapshotStreamMessage::Event {
                    rev: 0,
                    event: Box::new(event),
                })
                .await?;
            }
            Ok(ReplayOutcome::Replay { last_sent })
        }
        WorkspaceSessionReplay::ResetRequired => Ok(ReplayOutcome::ResetRequired),
    }
}

pub(super) fn primary_session_id_for_active_task(task: &WorkspaceActiveTaskSummary) -> SessionId {
    task.task
        .primary_session_id
        .unwrap_or(task.primary_session.session.id)
}

pub(super) fn session_ids_for_active_task_summary(
    task: &WorkspaceActiveTaskSummary,
) -> HashSet<SessionId> {
    let mut sessions = HashSet::new();
    sessions.insert(task.primary_session.session.id);
    for summary in &task.sessions {
        sessions.insert(summary.session.id);
    }
    sessions
}

async fn resolve_foreground_task_sessions(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    task_id: TaskId,
) -> HashSet<SessionId> {
    if let Some(summary) = state
        .workspaces
        .workspace_active_snapshot
        .active_task_summary(workspace_id, task_id)
        .await
    {
        return session_ids_for_active_task_summary(&summary);
    }
    let store = match state.store_for_workspace(workspace_id).await {
        Ok(store) => store,
        Err(_) => return HashSet::new(),
    };
    match store.get_workspace_active_task_summary(task_id).await {
        Ok(Some(summary)) => session_ids_for_active_task_summary(&summary),
        _ => HashSet::new(),
    }
}

pub(super) async fn resolve_workspace_active_snapshot_subscriptions(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    message: WorkspaceActiveSnapshotClientMessage,
    existing: &HashMap<SessionId, SessionCursor>,
) -> Result<ResolvedWorkspaceActiveSubscriptions, ()> {
    match message {
        WorkspaceActiveSnapshotClientMessage::Subscribe {
            session_ids,
            sessions,
            task_ids,
            foreground_task_id,
            scope,
            ..
        } => {
            let mut resolved = HashSet::new();
            let mut after_map: HashMap<SessionId, Option<i64>> = HashMap::new();
            let mut explicit_sessions = HashSet::new();
            let mut active_task_sessions = HashMap::new();
            let mut active_scope = false;
            let mut foreground_session_ids = None;
            for sub in sessions {
                after_map.insert(sub.session_id, sub.after_seq);
                resolved.insert(sub.session_id);
                explicit_sessions.insert(sub.session_id);
            }
            for session_id in session_ids {
                resolved.insert(session_id);
                explicit_sessions.insert(session_id);
            }
            if matches!(scope, Some(WorkspaceActiveSnapshotSubscribeScope::Active)) {
                active_scope = true;
                let snapshot = state
                    .workspaces
                    .workspace_active_snapshot
                    .active_snapshot(workspace_id, i64::MAX)
                    .await;
                for task in snapshot.active.tasks {
                    let session_id = primary_session_id_for_active_task(&task);
                    resolved.insert(session_id);
                    active_task_sessions.insert(task.task.id, session_id);
                }
            }
            if !task_ids.is_empty() {
                let store = state
                    .store_for_workspace(workspace_id)
                    .await
                    .map_err(|_| ())?;
                for task_id in task_ids {
                    let task = store.get_task(task_id).await.map_err(|_| ())?;
                    let Some(task) = task else {
                        continue;
                    };
                    if task.workspace_id != workspace_id {
                        continue;
                    }
                    if let Some(primary_session_id) = task.primary_session_id {
                        resolved.insert(primary_session_id);
                        explicit_sessions.insert(primary_session_id);
                    }
                }
            }
            if let Some(task_id) = foreground_task_id {
                let sessions = resolve_foreground_task_sessions(state, workspace_id, task_id).await;
                foreground_session_ids = Some(sessions);
            }

            let mut next: Vec<WorkspaceActiveSnapshotSessionSubscription> = resolved
                .into_iter()
                .map(|session_id| WorkspaceActiveSnapshotSessionSubscription {
                    session_id,
                    after_seq: after_map
                        .get(&session_id)
                        .copied()
                        .flatten()
                        .or_else(|| existing.get(&session_id).map(|cursor| cursor.last_sent)),
                })
                .collect();
            for sub in next.iter_mut() {
                if sub.after_seq.is_none() {
                    let last_seq = state
                        .workspaces
                        .workspace_active_snapshot
                        .session_last_event_seq(workspace_id, sub.session_id)
                        .await;
                    sub.after_seq = Some(last_seq);
                }
            }
            next.sort_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
            Ok(ResolvedWorkspaceActiveSubscriptions {
                sessions: next,
                state: WorkspaceActiveSubscriptionState {
                    active_scope,
                    explicit_sessions,
                    active_task_sessions,
                    foreground_task_id,
                    foreground_session_ids,
                },
            })
        }
    }
}

pub(super) async fn ensure_worktree_vcs_watchers_for_sessions(
    state: &Arc<AppState>,
    session_ids: &[SessionId],
) {
    if session_ids.is_empty() {
        return;
    }
    let mut worktrees: HashMap<WorktreeId, Worktree> = HashMap::new();
    for session_id in session_ids {
        let store = match state.store_for_session(*session_id).await {
            Ok(store) => store,
            Err(_) => continue,
        };
        let session = match store.get_session(*session_id).await {
            Ok(Some(session)) => session,
            _ => continue,
        };
        let worktree = match store.get_worktree(session.worktree_id).await {
            Ok(Some(worktree)) => worktree,
            _ => continue,
        };
        worktrees.entry(worktree.id).or_insert(worktree);
    }

    for (worktree_id, worktree) in worktrees {
        state.ensure_git_status_watcher(worktree.clone()).await;
        if let Err(err) =
            crate::git_status::emit_worktree_vcs_snapshot_for_worktree(state, &worktree, true).await
        {
            tracing::warn!(
                worktree_id = %worktree_id.0,
                "worktree vcs seed failed: {err:#}"
            );
        }
    }
}

pub(super) fn spawn_worktree_vcs_warmup_for_sessions(
    state: Arc<AppState>,
    session_ids: Vec<SessionId>,
) {
    if session_ids.is_empty() {
        return;
    }
    tokio::spawn(async move {
        ensure_worktree_vcs_watchers_for_sessions(&state, &session_ids).await;
    });
}

async fn resolve_worktree_ids_for_sessions(
    state: &Arc<AppState>,
    session_ids: &[SessionId],
) -> HashSet<WorktreeId> {
    let mut worktree_ids = HashSet::new();
    for session_id in session_ids {
        let store = match state.store_for_session(*session_id).await {
            Ok(store) => store,
            Err(_) => continue,
        };
        let session = match store.get_session(*session_id).await {
            Ok(Some(session)) => session,
            _ => continue,
        };
        worktree_ids.insert(session.worktree_id);
    }
    worktree_ids
}

pub(super) async fn sync_active_worktrees(
    state: &Arc<AppState>,
    active_worktrees: &mut HashSet<WorktreeId>,
    session_ids: &[SessionId],
) {
    let next = resolve_worktree_ids_for_sessions(state, session_ids).await;
    state
        .update_worktree_vcs_activity(active_worktrees, &next)
        .await;
    *active_worktrees = next;
}
