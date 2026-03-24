use super::*;

pub(super) enum ReplayOutcome {
    Replay { last_sent: SessionReplayCursor },
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
    worktree_vcs_session_ids: &[SessionId],
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
    let mut active_snapshot = state
        .workspaces
        .workspace_active_snapshot
        .active_snapshot(workspace_id, i64::MAX)
        .await;
    let extra_worktree_vcs_snapshots =
        load_worktree_vcs_snapshots_for_sessions(state, worktree_vcs_session_ids).await;
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

fn merge_worktree_vcs_snapshots(
    snapshots: Vec<WorktreeVcsSnapshot>,
    extras: Vec<WorktreeVcsSnapshot>,
) -> Vec<WorktreeVcsSnapshot> {
    let mut merged: HashMap<WorktreeId, WorktreeVcsSnapshot> = HashMap::new();
    for snapshot in snapshots {
        merged.insert(snapshot.worktree_id, snapshot);
    }
    for snapshot in extras {
        merged.insert(snapshot.worktree_id, snapshot);
    }
    let mut ordered: Vec<_> = merged.into_values().collect();
    ordered.sort_by_key(|snapshot| snapshot.worktree_id.0);
    ordered
}

pub(super) async fn replay_session_events<F, Fut>(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    after_cursor: SessionReplayCursor,
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
            after_cursor.last_event_seq,
            after_cursor.projection_rev,
            SESSION_REPLAY_MAX_EVENTS,
        )
        .await;
    match replay {
        WorkspaceSessionReplay::Replay {
            mut items,
            mut last_sent,
        } => {
            let saw_gap = items
                .iter()
                .any(|item| matches!(item, WorkspaceSessionReplayItem::Gap { .. }));
            let saw_seed = items
                .iter()
                .any(|item| matches!(item, WorkspaceSessionReplayItem::Seed(_)));
            if saw_gap && !saw_seed {
                let store = state.store_for_session(session_id).await.map_err(|_| ())?;
                if let Ok(Some(head)) = store.get_session_head_snapshot(session_id, 60, true).await
                {
                    state
                        .workspaces
                        .workspace_active_snapshot
                        .update_session_head(head.clone())
                        .await;
                    last_sent = SessionReplayCursor::from_head(&head);
                    items.push(WorkspaceSessionReplayItem::Seed(Box::new(head)));
                }
            }
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
                            delta,
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
                            head,
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

pub(super) fn resolve_worktree_vcs_interest_session_ids<I>(
    session_ids: I,
    subscription_state: &WorkspaceActiveSubscriptionState,
) -> Vec<SessionId>
where
    I: IntoIterator<Item = SessionId>,
{
    let mut ids: HashSet<SessionId> = session_ids.into_iter().collect();
    for task_session_ids in subscription_state.active_task_vcs_sessions.values() {
        ids.extend(task_session_ids.iter().copied());
    }
    let mut ordered: Vec<_> = ids.into_iter().collect();
    ordered.sort_by_key(|session_id| session_id.0);
    ordered
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

fn resolve_session_replay(
    replay: Option<&WorkspaceActiveSnapshotSessionReplay>,
    existing_last_sent: Option<SessionReplayCursor>,
    current_tail: SessionReplayCursor,
) -> ResolvedWorkspaceActiveSessionReplay {
    match replay {
        Some(WorkspaceActiveSnapshotSessionReplay::Resume {
            after_seq,
            after_projection_rev,
        }) => ResolvedWorkspaceActiveSessionReplay::Resume {
            after_seq: *after_seq,
            after_projection_rev: *after_projection_rev,
        },
        Some(WorkspaceActiveSnapshotSessionReplay::Reset) => {
            ResolvedWorkspaceActiveSessionReplay::Reset
        }
        Some(WorkspaceActiveSnapshotSessionReplay::Auto) | None => {
            let cursor = existing_last_sent.unwrap_or(current_tail);
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: cursor.last_event_seq,
                after_projection_rev: cursor.projection_rev,
            }
        }
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
            let mut replay_map: HashMap<SessionId, WorkspaceActiveSnapshotSessionReplay> =
                HashMap::new();
            let mut explicit_sessions = HashSet::new();
            let mut active_task_sessions = HashMap::new();
            let mut active_task_vcs_sessions = HashMap::new();
            let mut active_scope = false;
            let mut foreground_session_ids = None;
            for sub in sessions {
                replay_map.insert(sub.session_id, sub.replay);
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
                    let task_session_ids = session_ids_for_active_task_summary(&task);
                    let session_id = primary_session_id_for_active_task(&task);
                    resolved.insert(session_id);
                    active_task_sessions.insert(task.task.id, session_id);
                    active_task_vcs_sessions.insert(task.task.id, task_session_ids);
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

            let mut next = Vec::with_capacity(resolved.len());
            for session_id in resolved {
                let replay = replay_map.get(&session_id);
                let existing_last_sent = existing.get(&session_id).map(|cursor| cursor.last_sent);
                let current_tail = if matches!(
                    replay,
                    Some(WorkspaceActiveSnapshotSessionReplay::Auto) | None
                ) && existing_last_sent.is_none()
                {
                    state
                        .workspaces
                        .workspace_active_snapshot
                        .session_replay_cursor(workspace_id, session_id)
                        .await
                } else {
                    SessionReplayCursor::default()
                };
                let replay = resolve_session_replay(replay, existing_last_sent, current_tail);
                next.push(ResolvedWorkspaceActiveSessionSubscription { session_id, replay });
            }
            next.sort_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
            let state = WorkspaceActiveSubscriptionState {
                active_scope,
                explicit_sessions,
                active_task_sessions,
                active_task_vcs_sessions,
                foreground_task_id,
                foreground_session_ids,
            };
            let worktree_vcs_session_ids = resolve_worktree_vcs_interest_session_ids(
                next.iter().map(|sub| sub.session_id),
                &state,
            );
            Ok(ResolvedWorkspaceActiveSubscriptions {
                sessions: next,
                worktree_vcs_session_ids,
                state,
            })
        }
    }
}

pub(super) async fn refresh_worktree_vcs_for_sessions(
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
        match state.get_worktree_vcs_snapshot(worktree.id).await {
            Some(snapshot)
                if snapshot.freshness == WorktreeVcsFreshness::Fresh && snapshot.available => {}
            Some(_) => {
                crate::git_status::schedule_worktree_vcs_summary_refresh(
                    state.clone(),
                    worktree.clone(),
                )
                .await;
            }
            None => {
                if let Err(err) = crate::git_status::emit_worktree_vcs_snapshot_for_worktree(
                    state, &worktree, true,
                )
                .await
                {
                    tracing::warn!(
                        worktree_id = %worktree_id.0,
                        "worktree vcs seed failed: {err:#}"
                    );
                }
            }
        }
    }
}

pub(super) fn spawn_worktree_vcs_refresh_for_sessions(
    state: Arc<AppState>,
    session_ids: Vec<SessionId>,
) {
    if session_ids.is_empty() {
        return;
    }
    tokio::spawn(async move {
        refresh_worktree_vcs_for_sessions(&state, &session_ids).await;
    });
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

pub(super) async fn load_worktree_vcs_snapshots_for_sessions(
    state: &Arc<AppState>,
    session_ids: &[SessionId],
) -> Vec<WorktreeVcsSnapshot> {
    let worktree_ids = resolve_worktree_ids_for_sessions(state, session_ids).await;
    let mut ordered_worktree_ids: Vec<_> = worktree_ids.into_iter().collect();
    ordered_worktree_ids.sort_by_key(|worktree_id| worktree_id.0);
    let mut snapshots = Vec::new();
    for worktree_id in ordered_worktree_ids {
        if let Some(snapshot) = state.get_worktree_vcs_snapshot(worktree_id).await {
            snapshots.push(snapshot);
        }
    }
    snapshots
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

#[cfg(test)]
mod tests {
    use super::*;

    fn session_id(value: &str) -> SessionId {
        SessionId(uuid::Uuid::parse_str(value).unwrap())
    }

    #[test]
    fn replay_deserializes_subscribe_message_with_explicit_replay_modes() {
        let message = serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(
            r#"{
                "type":"subscribe",
                "sessions":[
                    {
                        "session_id":"00000000-0000-0000-0000-000000000001",
                        "replay":{"mode":"auto"}
                    },
                    {
                        "session_id":"00000000-0000-0000-0000-000000000002",
                        "replay":{"mode":"resume","after_seq":12}
                    },
                    {
                        "session_id":"00000000-0000-0000-0000-000000000003",
                        "replay":{"mode":"reset"}
                    }
                ]
            }"#,
        )
        .unwrap();

        let WorkspaceActiveSnapshotClientMessage::Subscribe { sessions, .. } = message;
        assert_eq!(sessions.len(), 3);
        assert!(matches!(
            sessions[0].replay,
            WorkspaceActiveSnapshotSessionReplay::Auto
        ));
        assert!(matches!(
            sessions[1].replay,
            WorkspaceActiveSnapshotSessionReplay::Resume {
                after_seq: 12,
                after_projection_rev: 0,
            }
        ));
        assert!(matches!(
            sessions[2].replay,
            WorkspaceActiveSnapshotSessionReplay::Reset
        ));
    }

    #[test]
    fn replay_resolution_uses_existing_cursor_for_auto() {
        let existing_last_sent = Some(SessionReplayCursor {
            last_event_seq: 7,
            projection_rev: 9,
        });
        let current_tail = SessionReplayCursor {
            last_event_seq: 41,
            projection_rev: 41,
        };

        assert!(matches!(
            resolve_session_replay(
                Some(&WorkspaceActiveSnapshotSessionReplay::Auto),
                existing_last_sent,
                current_tail,
            ),
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: 7,
                after_projection_rev: 9,
            }
        ));
        assert!(matches!(
            resolve_session_replay(None, existing_last_sent, current_tail),
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: 7,
                after_projection_rev: 9,
            }
        ));
    }

    #[test]
    fn replay_resolution_keeps_reset_explicit_even_with_existing_cursor() {
        assert!(matches!(
            resolve_session_replay(
                Some(&WorkspaceActiveSnapshotSessionReplay::Reset),
                Some(SessionReplayCursor {
                    last_event_seq: 7,
                    projection_rev: 7,
                }),
                SessionReplayCursor {
                    last_event_seq: 41,
                    projection_rev: 41,
                },
            ),
            ResolvedWorkspaceActiveSessionReplay::Reset
        ));
    }

    #[test]
    fn replay_resolution_uses_explicit_resume_cursor() {
        assert!(matches!(
            resolve_session_replay(
                Some(&WorkspaceActiveSnapshotSessionReplay::Resume {
                    after_seq: 19,
                    after_projection_rev: 23,
                }),
                Some(SessionReplayCursor {
                    last_event_seq: 7,
                    projection_rev: 7,
                }),
                SessionReplayCursor {
                    last_event_seq: 41,
                    projection_rev: 41,
                },
            ),
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: 19,
                after_projection_rev: 23,
            }
        ));
    }

    #[test]
    fn replay_resolution_uses_current_tail_for_auto_without_existing_cursor() {
        assert!(matches!(
            resolve_session_replay(
                Some(&WorkspaceActiveSnapshotSessionReplay::Auto),
                None,
                SessionReplayCursor {
                    last_event_seq: 41,
                    projection_rev: 43,
                },
            ),
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: 41,
                after_projection_rev: 43,
            }
        ));
        assert!(matches!(
            resolve_session_replay(
                None,
                None,
                SessionReplayCursor {
                    last_event_seq: 41,
                    projection_rev: 43,
                },
            ),
            ResolvedWorkspaceActiveSessionReplay::Resume {
                after_seq: 41,
                after_projection_rev: 43,
            }
        ));
    }

    #[test]
    fn replay_deserialization_keeps_session_ids_and_explicit_sessions_distinct() {
        let message = serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(
            r#"{
                "type":"subscribe",
                "session_ids":["00000000-0000-0000-0000-000000000001"],
                "sessions":[
                    {
                        "session_id":"00000000-0000-0000-0000-000000000002",
                        "replay":{"mode":"reset"}
                    }
                ]
            }"#,
        )
        .unwrap();

        let WorkspaceActiveSnapshotClientMessage::Subscribe {
            session_ids,
            sessions,
            ..
        } = message;
        assert_eq!(
            session_ids,
            vec![session_id("00000000-0000-0000-0000-000000000001")]
        );
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].session_id,
            session_id("00000000-0000-0000-0000-000000000002")
        );
        assert!(matches!(
            sessions[0].replay,
            WorkspaceActiveSnapshotSessionReplay::Reset
        ));
    }
}
