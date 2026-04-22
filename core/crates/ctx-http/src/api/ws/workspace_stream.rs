use super::replay::primary_session_id_for_active_task;
use super::*;
use serde_json::json;

pub(super) struct WorkspaceStreamRuntime {
    pub(super) priority_control: Arc<StreamQueue<WorkspaceActiveSnapshotStreamMessage>>,
    pub(super) control: Arc<StreamQueue<WorkspaceActiveSnapshotStreamMessage>>,
    pub(super) foreground_head_buffer: Arc<HeadBatchBuffer>,
    pub(super) background_head_buffer: Arc<HeadBatchBuffer>,
    pub(super) summary_buffer: Arc<SummaryBatchBuffer>,
    pub(super) send_control: Arc<StreamSendControl>,
    pub(super) subscriptions: HashMap<SessionId, SessionCursor>,
    pub(super) subscription_state: WorkspaceActiveSubscriptionState,
    pub(super) active_worktrees: HashSet<WorktreeId>,
    pub(super) open_worktrees: HashSet<WorktreeId>,
    pub(super) reset_queued: bool,
    pub(super) latest_snapshot_rev: Arc<AtomicI64>,
}

pub(super) struct WorkspaceStreamLabels {
    pub(super) ready_queue_label: &'static str,
    pub(super) subscribe_resolution_log: &'static str,
    pub(super) replay_list_metric: &'static str,
    pub(super) replay_send_metric: Option<&'static str>,
    pub(super) replay_queue_label: &'static str,
    pub(super) replay_failure_log: &'static str,
    pub(super) lagged_log: &'static str,
    pub(super) event_queue_label: &'static str,
}

async fn emit_workspace_stream_incident(
    state: &Arc<AppState>,
    event_name: &'static str,
    _workspace_id: WorkspaceId,
    labels: &[(&'static str, serde_json::Value)],
) {
    let mut event = crate::telemetry::TelemetryEvent::daemon_incident(event_name)
        .with_source("workspace_stream")
        .with_property("has_workspace_scope", json!(true));
    for (key, value) in labels {
        event = event.with_property(*key, value.clone());
    }
    state.telemetry.telemetry.emit(event).await;
}

pub(super) async fn initialize_workspace_stream(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    ready_queue_label: &'static str,
) -> Option<(
    WorkspaceStreamRuntime,
    tokio::sync::broadcast::Receiver<WorkspaceActiveSnapshotEvent>,
)> {
    let rx = state
        .workspaces
        .workspace_active_snapshot
        .subscribe(workspace_id)
        .await;
    let priority_control = Arc::new(StreamQueue::new(
        WORKSPACE_STREAM_QUEUE_LIMIT,
        WORKSPACE_STREAM_QUEUE_MAX_AGE,
    ));
    let control = Arc::new(StreamQueue::new(
        WORKSPACE_STREAM_QUEUE_LIMIT,
        WORKSPACE_STREAM_QUEUE_MAX_AGE,
    ));
    let foreground_head_buffer = Arc::new(HeadBatchBuffer::new());
    let background_head_buffer = Arc::new(HeadBatchBuffer::new());
    let summary_buffer = Arc::new(SummaryBatchBuffer::new(HEAD_BATCH_TOTAL_LIMIT));
    let send_control = Arc::new(StreamSendControl::new());
    let (snapshot_rev, archived_rev) =
        super::super::tasks::load_workspace_active_snapshot_state(state, workspace_id).await;
    let ready = WorkspaceActiveSnapshotEvent::Ready {
        workspace_id,
        snapshot_rev,
        archived_rev,
    };
    if push_stream_message(
        &control,
        workspace_id,
        None,
        ready_queue_label,
        WorkspaceActiveSnapshotStreamMessage::Event {
            rev: 0,
            event: Box::new(ready),
        },
    )
    .await
    .is_err()
    {
        return None;
    }

    Some((
        WorkspaceStreamRuntime {
            priority_control,
            control,
            foreground_head_buffer,
            background_head_buffer,
            summary_buffer,
            send_control,
            subscriptions: HashMap::new(),
            subscription_state: WorkspaceActiveSubscriptionState::default(),
            active_worktrees: HashSet::new(),
            open_worktrees: HashSet::new(),
            reset_queued: false,
            latest_snapshot_rev: Arc::new(AtomicI64::new(snapshot_rev)),
        },
        rx,
    ))
}

pub(super) async fn handle_workspace_stream_subscription(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    message: WorkspaceActiveSnapshotClientMessage,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
) -> Result<(), ()> {
    let include_initial_snapshot = matches!(
        &message,
        WorkspaceActiveSnapshotClientMessage::Subscribe {
            include_active_heads: true,
            ..
        }
    );
    state
        .ensure_workspace_active_snapshot_hydrated(workspace_id)
        .await
        .map_err(|error| {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                "workspace stream hydration failed: {error:?}"
            );
        })?;
    crate::merge_queue::activate_workspace_merge_queue(state, workspace_id).await;
    let existing_replay_cursors = runtime
        .subscriptions
        .iter()
        .map(|(session_id, cursor)| (*session_id, cursor.last_sent))
        .collect::<HashMap<_, _>>();
    let resolved = match resolve_workspace_active_snapshot_subscriptions(
        state,
        workspace_id,
        message,
        &existing_replay_cursors,
    )
    .await
    {
        Ok(next) => next,
        Err(_) => {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                "{}",
                labels.subscribe_resolution_log,
            );
            queue_workspace_stream_reset(state, workspace_id, runtime).await?;
            return Ok(());
        }
    };
    let ResolvedWorkspaceActiveSubscriptions {
        sessions: resolved_sessions,
        worktree_vcs_summary_session_ids,
        worktree_vcs_open_session_ids,
        state: next_state,
    } = resolved;

    clear_runtime_queues(runtime).await;
    runtime.reset_queued = false;
    runtime.send_control.clear_disconnect_after_flush();
    sync_active_worktrees(
        state,
        &mut runtime.active_worktrees,
        &mut runtime.open_worktrees,
        &worktree_vcs_summary_session_ids,
        &worktree_vcs_open_session_ids,
    )
    .await;
    let active_head_cursors = if include_initial_snapshot {
        runtime.send_control.set_hydrating();
        if queue_snapshot_payload(
            &runtime.control,
            state,
            workspace_id,
            &worktree_vcs_summary_session_ids,
            &worktree_vcs_open_session_ids,
        )
        .await
        .is_err()
        {
            return Err(());
        }
        state
            .workspaces
            .workspace_active_snapshot
            .active_heads(workspace_id)
            .await
            .heads
            .into_iter()
            .map(|head| (head.session.id, SessionReplayCursor::from_head(&head)))
            .collect::<HashMap<_, _>>()
    } else {
        HashMap::new()
    };

    let mut skip_replay_sessions = HashSet::new();
    if include_initial_snapshot && next_state.active_scope {
        for session_id in next_state.active_task_sessions.values() {
            let Some(snapshot_cursor) = active_head_cursors.get(session_id).copied() else {
                continue;
            };
            let current_tail = state
                .workspaces
                .workspace_active_snapshot
                .session_replay_cursor(workspace_id, *session_id)
                .await;
            if current_tail <= snapshot_cursor {
                skip_replay_sessions.insert(*session_id);
            }
        }
    }

    let mut next_map = HashMap::new();
    let mut replay_failed = false;
    for sub in &resolved_sessions {
        let session_id = sub.session_id;
        let ResolvedWorkspaceActiveSessionReplay::Resume {
            after_seq,
            after_projection_rev,
        } = sub.replay
        else {
            continue;
        };
        if include_initial_snapshot && skip_replay_sessions.contains(&session_id) {
            let last_sent = state
                .workspaces
                .workspace_active_snapshot
                .session_replay_cursor(workspace_id, session_id)
                .await;
            next_map.insert(
                session_id,
                SessionCursor {
                    last_sent: SessionReplayCursor {
                        last_event_seq: last_sent.last_event_seq.max(after_seq),
                        projection_rev: last_sent.projection_rev.max(after_projection_rev),
                    },
                },
            );
            continue;
        }
        let control = runtime.control.clone();
        let priority_control = runtime.priority_control.clone();
        let foreground_head_buffer = runtime.foreground_head_buffer.clone();
        let background_head_buffer = runtime.background_head_buffer.clone();
        let summary_buffer = runtime.summary_buffer.clone();
        let active_task_sessions = next_state.active_task_sessions.clone();
        let explicit_sessions = next_state.explicit_sessions.clone();
        let foreground_session_ids = next_state.foreground_session_ids.clone();
        let replay = replay_session_events(
            state,
            workspace_id,
            session_id,
            SessionReplayCursor {
                last_event_seq: after_seq,
                projection_rev: after_projection_rev,
            },
            labels.replay_list_metric,
            labels.replay_send_metric,
            move |event| {
                let control = control.clone();
                let priority_control = priority_control.clone();
                let foreground_head_buffer = foreground_head_buffer.clone();
                let background_head_buffer = background_head_buffer.clone();
                let summary_buffer = summary_buffer.clone();
                let active_task_sessions = active_task_sessions.clone();
                let explicit_sessions = explicit_sessions.clone();
                let foreground_session_ids = foreground_session_ids.clone();
                async move {
                    match event {
                        WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => match *event {
                            WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                                snapshot_rev,
                                delta,
                                ..
                            } => {
                                if !should_stream_head_delta(
                                    &active_task_sessions,
                                    &explicit_sessions,
                                    foreground_session_ids.as_ref(),
                                    delta.session_id,
                                ) {
                                    return Ok(());
                                }
                                let Some(delta) = filter_partial_delta_for_active_tasks(
                                    *delta,
                                    &active_task_sessions,
                                    foreground_session_ids.as_ref(),
                                ) else {
                                    return Ok(());
                                };
                                let head_buffer = if is_foreground_session(
                                    foreground_session_ids.as_ref(),
                                    delta.session_id,
                                ) {
                                    &foreground_head_buffer
                                } else {
                                    &background_head_buffer
                                };
                                if let Err(error) = head_buffer.push(snapshot_rev, delta).await {
                                    log_head_batch_push_error(
                                        labels.replay_queue_label,
                                        workspace_id,
                                        &error,
                                    );
                                    return Err(());
                                }
                                Ok(())
                            }
                            other @ WorkspaceActiveSnapshotEvent::SessionSummaryDelta { .. } => {
                                summary_buffer.push(other).await.map_err(|error| {
                                    log_summary_batch_push_error(
                                        labels.replay_queue_label,
                                        workspace_id,
                                        &error,
                                    );
                                })
                            }
                            other => {
                                let target = if is_priority_control_event(
                                    &other,
                                    foreground_session_ids.as_ref(),
                                ) {
                                    &priority_control
                                } else {
                                    &control
                                };
                                push_stream_message(
                                    target,
                                    workspace_id,
                                    Some(session_id),
                                    labels.replay_queue_label,
                                    WorkspaceActiveSnapshotStreamMessage::Event {
                                        rev: 0,
                                        event: Box::new(other),
                                    },
                                )
                                .await
                            }
                        },
                        other => {
                            push_stream_message(
                                &control,
                                workspace_id,
                                Some(session_id),
                                labels.replay_queue_label,
                                other,
                            )
                            .await
                        }
                    }
                }
            },
        )
        .await;
        match replay {
            Ok(ReplayOutcome::Replay { last_sent }) => {
                next_map.insert(session_id, SessionCursor { last_sent });
            }
            Ok(ReplayOutcome::ResetRequired) | Err(_) => {
                tracing::error!(
                    target: "ctx_http.ws_active_snapshot",
                    workspace_id = %workspace_id.0,
                    session_id = %session_id.0,
                    after_seq,
                    after_projection_rev,
                    "{}",
                    labels.replay_failure_log,
                );
                replay_failed = true;
                break;
            }
        };
    }
    if replay_failed {
        queue_workspace_stream_reset(state, workspace_id, runtime).await?;
        return Ok(());
    }
    sync_workspace_stream_session_pins(
        state,
        runtime.subscriptions.keys().copied(),
        next_map.keys().copied(),
    )
    .await;
    runtime.subscriptions = next_map;
    runtime.subscription_state = next_state;
    if seed_worktree_vcs_for_subscribe(
        &runtime.control,
        state,
        workspace_id,
        &worktree_vcs_summary_session_ids,
        if include_initial_snapshot {
            WorktreeVcsSeedMode::IncludedInSnapshot
        } else {
            WorktreeVcsSeedMode::EmitCachedEvents
        },
    )
    .await
    .is_err()
    {
        return Err(());
    }
    spawn_worktree_vcs_refresh_for_sessions(
        state.clone(),
        worktree_vcs_summary_session_ids,
        worktree_vcs_open_session_ids,
    );
    Ok(())
}

pub(super) async fn handle_workspace_stream_lagged(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    lagged: u64,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
) -> Result<(), ()> {
    if runtime.reset_queued {
        return Ok(());
    }
    tracing::error!(
        target: "ctx_http.ws_active_snapshot",
        workspace_id = %workspace_id.0,
        lagged,
        "{}",
        labels.lagged_log,
    );
    emit_workspace_stream_incident(
        state,
        "workspace_stream_lagged",
        workspace_id,
        &[
            ("lagged", json!(lagged)),
            ("queue_label", json!(labels.event_queue_label)),
        ],
    )
    .await;
    queue_workspace_stream_reset(state, workspace_id, runtime).await
}

pub(super) async fn handle_workspace_stream_event(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    event: WorkspaceActiveSnapshotEvent,
    runtime: &mut WorkspaceStreamRuntime,
    labels: &WorkspaceStreamLabels,
) -> Result<(), ()> {
    if let Some(rev) = event_snapshot_rev(&event) {
        bump_latest_snapshot_rev(&runtime.latest_snapshot_rev, rev);
    }

    if runtime.reset_queued {
        return Ok(());
    }

    let mut refresh_active_worktrees = false;
    if runtime.subscription_state.active_scope {
        match &event {
            WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { task, .. } => {
                let session_id = primary_session_id_for_active_task(task);
                runtime
                    .subscription_state
                    .active_task_sessions
                    .insert(task.task.id, session_id);
                runtime.subscription_state.active_task_vcs_sessions.insert(
                    task.task.id,
                    primary_session_ids_for_active_task_summary(task),
                );
                refresh_active_worktrees = true;
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    runtime.subscriptions.entry(session_id)
                {
                    let last_sent = state
                        .workspaces
                        .workspace_active_snapshot
                        .session_replay_cursor(workspace_id, session_id)
                        .await;
                    entry.insert(SessionCursor { last_sent });
                }
            }
            WorkspaceActiveSnapshotEvent::ActiveTaskDelete { task_id, .. } => {
                let removed_vcs_sessions = runtime
                    .subscription_state
                    .active_task_vcs_sessions
                    .remove(task_id);
                if let Some(session_id) = runtime
                    .subscription_state
                    .active_task_sessions
                    .remove(task_id)
                {
                    refresh_active_worktrees = true;
                    let still_active = runtime
                        .subscription_state
                        .active_task_sessions
                        .values()
                        .any(|id| *id == session_id);
                    if !still_active
                        && !runtime
                            .subscription_state
                            .explicit_sessions
                            .contains(&session_id)
                    {
                        runtime.subscriptions.remove(&session_id);
                    }
                }
                if removed_vcs_sessions.is_some() {
                    refresh_active_worktrees = true;
                }
            }
            WorkspaceActiveSnapshotEvent::TaskDelta { delta, .. }
                if matches!(delta.kind, TaskDeltaKind::Archived) =>
            {
                let removed_vcs_sessions = runtime
                    .subscription_state
                    .active_task_vcs_sessions
                    .remove(&delta.task.id);
                if let Some(session_id) = runtime
                    .subscription_state
                    .active_task_sessions
                    .remove(&delta.task.id)
                {
                    refresh_active_worktrees = true;
                    let still_active = runtime
                        .subscription_state
                        .active_task_sessions
                        .values()
                        .any(|id| *id == session_id);
                    if !still_active
                        && !runtime
                            .subscription_state
                            .explicit_sessions
                            .contains(&session_id)
                    {
                        runtime.subscriptions.remove(&session_id);
                    }
                }
                if removed_vcs_sessions.is_some() {
                    refresh_active_worktrees = true;
                }
            }
            _ => {}
        }
    }
    if refresh_active_worktrees {
        let summary_session_ids = resolve_worktree_vcs_summary_session_ids(
            runtime.subscriptions.keys().copied(),
            &runtime.subscription_state,
        );
        let open_session_ids = resolve_worktree_vcs_open_session_ids(&runtime.subscription_state);
        sync_active_worktrees(
            state,
            &mut runtime.active_worktrees,
            &mut runtime.open_worktrees,
            &summary_session_ids,
            &open_session_ids,
        )
        .await;
        refresh_worktree_vcs_for_sessions(state, &summary_session_ids, &open_session_ids).await;
    }

    match &event {
        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
            let Some(cursor) = runtime.subscriptions.get_mut(&delta.session_id) else {
                return Ok(());
            };
            if !accept_session_delta(cursor, delta) {
                return Ok(());
            }
        }
        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
            let Some(cursor) = runtime.subscriptions.get_mut(&head.session.id) else {
                return Ok(());
            };
            if !accept_session_head(cursor, head) {
                return Ok(());
            }
        }
        _ => {}
    }

    let session_id = match &event {
        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => Some(delta.session_id),
        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => Some(head.session.id),
        WorkspaceActiveSnapshotEvent::SessionGap { session_id, .. } => Some(*session_id),
        WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. } => Some(delta.session_id),
        _ => None,
    };

    match event {
        WorkspaceActiveSnapshotEvent::SessionHeadDelta {
            snapshot_rev,
            delta,
            ..
        } => {
            if !should_stream_head_delta(
                &runtime.subscription_state.active_task_sessions,
                &runtime.subscription_state.explicit_sessions,
                runtime.subscription_state.foreground_session_ids.as_ref(),
                delta.session_id,
            ) {
                return Ok(());
            }
            let Some(delta) = filter_partial_delta_for_active_tasks(
                *delta,
                &runtime.subscription_state.active_task_sessions,
                runtime.subscription_state.foreground_session_ids.as_ref(),
            ) else {
                return Ok(());
            };
            let head_buffer = if is_foreground_session(
                runtime.subscription_state.foreground_session_ids.as_ref(),
                delta.session_id,
            ) {
                &runtime.foreground_head_buffer
            } else {
                &runtime.background_head_buffer
            };
            if let Err(error) = head_buffer.push(snapshot_rev, delta).await {
                log_head_batch_push_error(labels.event_queue_label, workspace_id, &error);
                if runtime.reset_queued {
                    return Ok(());
                }
                queue_workspace_stream_reset(state, workspace_id, runtime).await?;
            }
        }
        other @ WorkspaceActiveSnapshotEvent::SessionSummaryDelta { .. } => {
            if let Err(error) = runtime.summary_buffer.push(other).await {
                log_summary_batch_push_error(labels.event_queue_label, workspace_id, &error);
                if runtime.reset_queued {
                    return Ok(());
                }
                queue_workspace_stream_reset(state, workspace_id, runtime).await?;
            }
        }
        other => {
            let target = if is_priority_control_event(
                &other,
                runtime.subscription_state.foreground_session_ids.as_ref(),
            ) {
                &runtime.priority_control
            } else {
                &runtime.control
            };
            if push_stream_message(
                target,
                workspace_id,
                session_id,
                labels.event_queue_label,
                WorkspaceActiveSnapshotStreamMessage::Event {
                    rev: 0,
                    event: Box::new(other),
                },
            )
            .await
            .is_err()
            {
                if runtime.reset_queued {
                    return Ok(());
                }
                queue_workspace_stream_reset(state, workspace_id, runtime).await?;
            }
        }
    }

    Ok(())
}

pub(super) async fn notify_workspace_stream_shutdown(runtime: &WorkspaceStreamRuntime) {
    runtime.send_control.set_disconnect_after_flush();
    runtime.priority_control.notify.notify_one();
    runtime.control.notify.notify_one();
    runtime.foreground_head_buffer.notify.notify_one();
    runtime.background_head_buffer.notify.notify_one();
    runtime.summary_buffer.notify.notify_one();
}

pub(super) async fn release_workspace_stream(
    state: &Arc<AppState>,
    runtime: &WorkspaceStreamRuntime,
) {
    release_workspace_stream_session_pins(state, runtime.subscriptions.keys().copied()).await;
    state
        .update_worktree_vcs_activity(&runtime.active_worktrees, &HashSet::new())
        .await;
    state
        .update_worktree_vcs_open_panes(&runtime.open_worktrees, &HashSet::new())
        .await;
}

async fn clear_runtime_queues(runtime: &WorkspaceStreamRuntime) {
    runtime.priority_control.clear().await;
    runtime.control.clear().await;
    runtime.foreground_head_buffer.clear().await;
    runtime.background_head_buffer.clear().await;
    runtime.summary_buffer.clear().await;
}

async fn queue_workspace_stream_reset(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    runtime: &mut WorkspaceStreamRuntime,
) -> Result<(), ()> {
    clear_runtime_queues(runtime).await;
    if queue_reset_required(&runtime.priority_control, state, workspace_id)
        .await
        .is_err()
    {
        return Err(());
    }
    emit_workspace_stream_incident(
        state,
        "workspace_stream_reset_queued",
        workspace_id,
        &[(
            "latest_snapshot_rev",
            json!(runtime.latest_snapshot_rev.load(Ordering::Relaxed)),
        )],
    )
    .await;
    runtime.reset_queued = true;
    runtime.send_control.set_disconnect_after_flush();
    Ok(())
}
