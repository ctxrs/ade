use super::*;

pub(crate) async fn workspace_active_snapshot_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workspace_id = match uuid::Uuid::parse_str(&id) {
        Ok(v) => WorkspaceId(v),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    ws.on_upgrade(move |socket| handle_workspace_active_snapshot_ws(socket, state, workspace_id))
}

async fn handle_workspace_active_snapshot_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    workspace_id: WorkspaceId,
) {
    let (sender, mut receiver) = socket.split();
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
    let send_control = Arc::new(StreamSendControl::new());
    let mut rx = state
        .workspaces
        .workspace_active_snapshot
        .subscribe(workspace_id)
        .await;
    let mut subscriptions: HashMap<SessionId, SessionCursor> = HashMap::new();
    let mut subscription_state = WorkspaceActiveSubscriptionState::default();
    let mut active_worktrees: HashSet<WorktreeId> = HashSet::new();
    let mut reset_queued = false;

    let (snapshot_rev, archived_rev) =
        super::super::tasks::load_workspace_active_snapshot_state(&state, workspace_id).await;
    let ready = WorkspaceActiveSnapshotEvent::Ready {
        workspace_id,
        snapshot_rev,
        archived_rev,
    };
    if push_stream_message(
        &control,
        workspace_id,
        None,
        "ready",
        WorkspaceActiveSnapshotStreamMessage::Event {
            rev: 0,
            event: Box::new(ready),
        },
    )
    .await
    .is_err()
    {
        return;
    }

    let latest_snapshot_rev = Arc::new(AtomicI64::new(snapshot_rev));

    let send_task = {
        let priority_control = priority_control.clone();
        let control = control.clone();
        let foreground_head_buffer = foreground_head_buffer.clone();
        let background_head_buffer = background_head_buffer.clone();
        let send_control = send_control.clone();
        let latest_snapshot_rev = latest_snapshot_rev.clone();
        tokio::spawn(async move {
            let mut sender = sender;
            let mut stream_seq: i64 = 0;
            let mut tick = tokio::time::interval(HEAD_BATCH_FLUSH_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                if let Some(next) = take_next_workspace_stream_item(
                    &priority_control,
                    &control,
                    &foreground_head_buffer,
                    &background_head_buffer,
                    send_control.is_hydrating(),
                )
                .await
                {
                    match next {
                        NextWorkspaceStreamItem::Control(entry) => {
                            let message = entry.message;
                            let queued_ms = entry.enqueued_at.elapsed().as_millis();
                            let is_snapshot = matches!(
                                message,
                                WorkspaceActiveSnapshotStreamMessage::Snapshot { .. }
                            );
                            let message = match message {
                                WorkspaceActiveSnapshotStreamMessage::ResetRequired { .. } => {
                                    message
                                }
                                other => {
                                    stream_seq += 1;
                                    with_stream_rev(other, stream_seq)
                                }
                            };
                            let serialize_start = Instant::now();
                            let Ok(text) = serde_json::to_string(&message) else {
                                break;
                            };
                            let payload_bytes = text.len();
                            let send_start = Instant::now();
                            if sender.send(WsMessage::Text(text)).await.is_err() {
                                break;
                            }
                            if is_snapshot {
                                let encode_ms = serialize_start.elapsed().as_millis();
                                let send_ms = send_start.elapsed().as_millis();
                                let (task_count, head_count) = match &message {
                                    WorkspaceActiveSnapshotStreamMessage::Snapshot {
                                        active_snapshot,
                                        active_heads,
                                        ..
                                    } => (
                                        active_snapshot.active.tasks.len(),
                                        active_heads.as_ref().map(|h| h.heads.len()).unwrap_or(0),
                                    ),
                                    _ => (0, 0),
                                };
                                tracing::info!(
                                    target: "ctx_http.ws_active_snapshot",
                                    workspace_id = %workspace_id.0,
                                    snapshot_bytes = payload_bytes,
                                    snapshot_queue_ms = queued_ms,
                                    snapshot_encode_ms = encode_ms,
                                    snapshot_send_ms = send_ms,
                                    active_tasks = task_count,
                                    active_heads = head_count,
                                    "workspace snapshot sent",
                                );
                            }
                            if is_snapshot {
                                send_control.clear_hydrating();
                            }
                        }
                        NextWorkspaceStreamItem::HeadsBatch {
                            snapshot_rev,
                            deltas,
                        } => {
                            let latest_rev = latest_snapshot_rev.load(Ordering::Relaxed);
                            let snapshot_rev = snapshot_rev.max(latest_rev);
                            bump_latest_snapshot_rev(&latest_snapshot_rev, snapshot_rev);
                            stream_seq += 1;
                            let message = WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
                                rev: stream_seq,
                                snapshot_rev,
                                deltas,
                            };
                            let Ok(text) = serde_json::to_string(&message) else {
                                break;
                            };
                            if sender.send(WsMessage::Text(text)).await.is_err() {
                                break;
                            }
                        }
                    }
                    if send_control.should_disconnect_after_flush()
                        && workspace_stream_is_idle(
                            &priority_control,
                            &control,
                            &foreground_head_buffer,
                            &background_head_buffer,
                        )
                        .await
                    {
                        break;
                    }
                    continue;
                }
                if send_control.should_disconnect_after_flush() {
                    break;
                }

                tokio::select! {
                    _ = priority_control.notify.notified() => {},
                    _ = control.notify.notified() => {},
                    _ = foreground_head_buffer.notify.notified() => {},
                    _ = background_head_buffer.notify.notified() => {},
                    _ = tick.tick() => {},
                }
            }
        })
    };
    let recv_loop = async {
        loop {
            tokio::select! {
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(WsMessage::Text(text))) => {
                            if let Ok(message) =
                                serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(&text)
                            {
                                let mut subscribe_ctx = WorkspaceActiveSubscribeContext {
                                    priority_control: &priority_control,
                                    control: &control,
                                    foreground_head_buffer: &foreground_head_buffer,
                                    background_head_buffer: &background_head_buffer,
                                    send_control: &send_control,
                                    subscriptions: &mut subscriptions,
                                    subscription_state: &mut subscription_state,
                                    active_worktrees: &mut active_worktrees,
                                    reset_queued: &mut reset_queued,
                                };
                                handle_subscribe_message(
                                    &state,
                                    workspace_id,
                                    message,
                                    &mut subscribe_ctx,
                                ).await?;
                            }
                        }
                        Some(Ok(WsMessage::Binary(bytes))) => {
                            if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                                if let Ok(message) =
                                    serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(&text)
                                {
                                    let mut subscribe_ctx = WorkspaceActiveSubscribeContext {
                                        priority_control: &priority_control,
                                        control: &control,
                                        foreground_head_buffer: &foreground_head_buffer,
                                        background_head_buffer: &background_head_buffer,
                                        send_control: &send_control,
                                        subscriptions: &mut subscriptions,
                                        subscription_state: &mut subscription_state,
                                        active_worktrees: &mut active_worktrees,
                                        reset_queued: &mut reset_queued,
                                    };
                                    handle_subscribe_message(
                                        &state,
                                        workspace_id,
                                        message,
                                        &mut subscribe_ctx,
                                    ).await?;
                                }
                            }
                        }
                        Some(Ok(WsMessage::Close(_))) => break,
                        Some(Ok(_)) => {}
                        Some(Err(_)) => break,
                        None => break,
                    }
                }
                event = rx.recv() => {
                    let event = match event {
                        Ok(event) => event,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(lagged)) => {
                            if reset_queued {
                                continue;
                            }
                            tracing::error!(
                                target: "ctx_http.ws_active_snapshot",
                                workspace_id = %workspace_id.0,
                                lagged,
                                "workspace stream lagged",
                            );
                            priority_control.clear().await;
                            control.clear().await;
                            foreground_head_buffer.clear().await;
                            background_head_buffer.clear().await;
                            if queue_reset_required(&priority_control, &state, workspace_id)
                                .await
                                .is_err()
                            {
                                break;
                            }
                            reset_queued = true;
                            send_control.set_disconnect_after_flush();
                            continue;
                        }
                        Err(_) => break,
                    };

                    if let Some(rev) = event_snapshot_rev(&event) {
                        bump_latest_snapshot_rev(&latest_snapshot_rev, rev);
                    }

                    if reset_queued {
                        continue;
                    }

                    let mut refresh_active_worktrees = false;
                    if subscription_state.active_scope {
                        match &event {
                            WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { task, .. } => {
                                let task_session_ids = session_ids_for_active_task_summary(task);
                                let session_id = task
                                    .task
                                    .primary_session_id
                                    .unwrap_or(task.primary_session.session.id);
                                subscription_state
                                    .active_task_sessions
                                    .insert(task.task.id, session_id);
                                subscription_state
                                    .active_task_vcs_sessions
                                    .insert(task.task.id, task_session_ids);
                                refresh_active_worktrees = true;
                                if let std::collections::hash_map::Entry::Vacant(entry) =
                                    subscriptions.entry(session_id)
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
                                let removed_vcs_sessions =
                                    subscription_state.active_task_vcs_sessions.remove(task_id);
                                if let Some(session_id) =
                                    subscription_state.active_task_sessions.remove(task_id)
                                {
                                    refresh_active_worktrees = true;
                                    let still_active = subscription_state
                                        .active_task_sessions
                                        .values()
                                        .any(|id| *id == session_id);
                                    if !still_active
                                        && !subscription_state
                                            .explicit_sessions
                                            .contains(&session_id)
                                    {
                                            subscriptions.remove(&session_id);
                                    }
                                }
                                if removed_vcs_sessions.is_some() {
                                    refresh_active_worktrees = true;
                                }
                            }
                            WorkspaceActiveSnapshotEvent::TaskDelta { delta, .. }
                                if matches!(delta.kind, TaskDeltaKind::Archived) =>
                            {
                                let removed_vcs_sessions = subscription_state
                                    .active_task_vcs_sessions
                                    .remove(&delta.task.id);
                                if let Some(session_id) =
                                    subscription_state.active_task_sessions.remove(&delta.task.id)
                                {
                                    refresh_active_worktrees = true;
                                    let still_active = subscription_state
                                        .active_task_sessions
                                        .values()
                                        .any(|id| *id == session_id);
                                    if !still_active
                                        && !subscription_state
                                            .explicit_sessions
                                            .contains(&session_id)
                                    {
                                            subscriptions.remove(&session_id);
                                    }
                                }
                                if removed_vcs_sessions.is_some() {
                                    refresh_active_worktrees = true;
                                }
                            }
                            WorkspaceActiveSnapshotEvent::SessionSummary { summary, .. } => {
                                if let Some(ids) = subscription_state
                                    .active_task_vcs_sessions
                                    .get_mut(&summary.session.task_id)
                                {
                                    if ids.insert(summary.session.id) {
                                        refresh_active_worktrees = true;
                                    }
                                }
                            }
                            WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. } => {
                                if let Some(ids) = subscription_state
                                    .active_task_vcs_sessions
                                    .get_mut(&delta.task_id)
                                {
                                    if ids.insert(delta.session_id) {
                                        refresh_active_worktrees = true;
                                    }
                                }
                            }
                            WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
                                if let Some(ids) = subscription_state
                                    .active_task_vcs_sessions
                                    .get_mut(&head.session.task_id)
                                {
                                    if ids.insert(head.session.id) {
                                        refresh_active_worktrees = true;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    if refresh_active_worktrees {
                        let session_ids = resolve_worktree_vcs_interest_session_ids(
                            subscriptions.keys().copied(),
                            &subscription_state,
                        );
                        sync_active_worktrees(
                            &state,
                            &mut active_worktrees,
                            &session_ids,
                        )
                        .await;
                        refresh_worktree_vcs_for_sessions(
                            &state,
                            &session_ids,
                        )
                        .await;
                    }

                    match &event {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
                            let Some(cursor) = subscriptions.get_mut(&delta.session_id) else {
                                continue;
                            };
                            if !accept_session_delta(cursor, delta) {
                                continue;
                            }
                        }
                        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
                            let Some(cursor) = subscriptions.get_mut(&head.session.id) else {
                                continue;
                            };
                            if !accept_session_head(cursor, head) {
                                continue;
                            }
                        }
                        _ => {}
                    }

                    let session_id = match &event {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
                            Some(delta.session_id)
                        }
                        WorkspaceActiveSnapshotEvent::SessionHeadSeed { head, .. } => {
                            Some(head.session.id)
                        }
                        WorkspaceActiveSnapshotEvent::SessionGap { session_id, .. } => {
                            Some(*session_id)
                        }
                        WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. } => {
                            Some(delta.session_id)
                        }
                        _ => None,
                    };
                    match event {
                        WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                            snapshot_rev,
                            delta,
                            ..
                        } => {
                            let Some(delta) = filter_partial_delta_for_active_tasks(
                                *delta,
                                &subscription_state.active_task_sessions,
                                subscription_state.foreground_session_ids.as_ref(),
                            ) else {
                                continue;
                            };
                            let head_buffer = if is_foreground_session(
                                subscription_state.foreground_session_ids.as_ref(),
                                delta.session_id,
                            ) {
                                &foreground_head_buffer
                            } else {
                                &background_head_buffer
                            };
                            if let Err(err) = head_buffer.push(snapshot_rev, delta).await {
                                log_head_batch_push_error(
                                    "event",
                                    workspace_id,
                                    &err,
                                );
                                if reset_queued {
                                    continue;
                                }
                                priority_control.clear().await;
                                control.clear().await;
                                foreground_head_buffer.clear().await;
                                background_head_buffer.clear().await;
                                if queue_reset_required(&priority_control, &state, workspace_id)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                reset_queued = true;
                                send_control.set_disconnect_after_flush();
                            }
                        }
                        other => {
                            let target = if is_priority_control_event(
                                &other,
                                subscription_state.foreground_session_ids.as_ref(),
                            ) {
                                &priority_control
                            } else {
                                &control
                            };
                            if push_stream_message(
                                target,
                                workspace_id,
                                session_id,
                                "event",
                                WorkspaceActiveSnapshotStreamMessage::Event {
                                    rev: 0,
                                    event: Box::new(other),
                                },
                            )
                            .await
                            .is_err()
                            {
                                if reset_queued {
                                    continue;
                                }
                                priority_control.clear().await;
                                control.clear().await;
                                foreground_head_buffer.clear().await;
                                background_head_buffer.clear().await;
                                if queue_reset_required(&priority_control, &state, workspace_id)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                reset_queued = true;
                                send_control.set_disconnect_after_flush();
                            }
                        }
                    }
                }
            }
        }
        Ok::<(), ()>(())
    };

    let (send_task, _recv_result) = crate::async_util::race_join_handle(send_task, recv_loop).await;

    send_control.set_disconnect_after_flush();
    priority_control.notify.notify_one();
    control.notify.notify_one();
    foreground_head_buffer.notify.notify_one();
    background_head_buffer.notify.notify_one();
    if let Some(send_task) = send_task {
        let _ = send_task.await;
    }

    release_workspace_stream_session_pins(&state, subscriptions.keys().copied()).await;
    state
        .update_worktree_vcs_activity(&active_worktrees, &HashSet::new())
        .await;
}

struct WorkspaceActiveSubscribeContext<'a> {
    priority_control: &'a Arc<StreamQueue<WorkspaceActiveSnapshotStreamMessage>>,
    control: &'a Arc<StreamQueue<WorkspaceActiveSnapshotStreamMessage>>,
    foreground_head_buffer: &'a Arc<HeadBatchBuffer>,
    background_head_buffer: &'a Arc<HeadBatchBuffer>,
    send_control: &'a Arc<StreamSendControl>,
    subscriptions: &'a mut HashMap<SessionId, SessionCursor>,
    subscription_state: &'a mut WorkspaceActiveSubscriptionState,
    active_worktrees: &'a mut HashSet<WorktreeId>,
    reset_queued: &'a mut bool,
}

async fn handle_subscribe_message(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    message: WorkspaceActiveSnapshotClientMessage,
    ctx: &mut WorkspaceActiveSubscribeContext<'_>,
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
        .map_err(|err| {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                "workspace stream hydration failed: {err:?}"
            );
        })?;
    crate::merge_queue::activate_workspace_merge_queue(state, workspace_id).await;
    let resolved = match resolve_workspace_active_snapshot_subscriptions(
        state,
        workspace_id,
        message,
        ctx.subscriptions,
    )
    .await
    {
        Ok(next) => next,
        Err(_) => {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                "workspace stream subscribe resolution failed",
            );
            ctx.priority_control.clear().await;
            ctx.control.clear().await;
            ctx.foreground_head_buffer.clear().await;
            ctx.background_head_buffer.clear().await;
            if queue_reset_required(ctx.priority_control, state, workspace_id)
                .await
                .is_err()
            {
                return Err(());
            }
            *ctx.reset_queued = true;
            ctx.send_control.set_disconnect_after_flush();
            return Ok(());
        }
    };
    let ResolvedWorkspaceActiveSubscriptions {
        sessions: resolved_sessions,
        worktree_vcs_session_ids,
        state: next_state,
    } = resolved;

    ctx.priority_control.clear().await;
    ctx.control.clear().await;
    ctx.foreground_head_buffer.clear().await;
    ctx.background_head_buffer.clear().await;
    *ctx.reset_queued = false;
    ctx.send_control.clear_disconnect_after_flush();
    sync_active_worktrees(state, ctx.active_worktrees, &worktree_vcs_session_ids).await;
    let active_head_cursors = if include_initial_snapshot {
        ctx.send_control.set_hydrating();
        if queue_snapshot_payload(ctx.control, state, workspace_id, &worktree_vcs_session_ids)
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
        // Reset intentionally leaves the session quiet until the client resubscribes with resume.
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
        let control = ctx.control.clone();
        let priority_control = ctx.priority_control.clone();
        let foreground_head_buffer = ctx.foreground_head_buffer.clone();
        let background_head_buffer = ctx.background_head_buffer.clone();
        let active_task_sessions = next_state.active_task_sessions.clone();
        let foreground_session_ids = next_state.foreground_session_ids.clone();
        let replay = replay_session_events(
            state,
            workspace_id,
            session_id,
            SessionReplayCursor {
                last_event_seq: after_seq,
                projection_rev: after_projection_rev,
            },
            "ctx_http.replay_session_events_active.list",
            Some("ctx_http.replay_session_events_active.send"),
            move |event| {
                let control = control.clone();
                let priority_control = priority_control.clone();
                let foreground_head_buffer = foreground_head_buffer.clone();
                let background_head_buffer = background_head_buffer.clone();
                let active_task_sessions = active_task_sessions.clone();
                let foreground_session_ids = foreground_session_ids.clone();
                async move {
                    match event {
                        WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => match *event {
                            WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                                snapshot_rev,
                                delta,
                                ..
                            } => {
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
                                if let Err(err) = head_buffer.push(snapshot_rev, delta).await {
                                    log_head_batch_push_error("replay", workspace_id, &err);
                                    return Err(());
                                }
                                Ok(())
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
                                    "replay",
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
                                "replay",
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
                    "workspace stream replay failed",
                );
                replay_failed = true;
                break;
            }
        };
    }
    if replay_failed {
        ctx.priority_control.clear().await;
        ctx.control.clear().await;
        ctx.foreground_head_buffer.clear().await;
        ctx.background_head_buffer.clear().await;
        if queue_reset_required(ctx.priority_control, state, workspace_id)
            .await
            .is_err()
        {
            return Err(());
        }
        *ctx.reset_queued = true;
        ctx.send_control.set_disconnect_after_flush();
        return Ok(());
    }
    sync_workspace_stream_session_pins(
        state,
        ctx.subscriptions.keys().copied(),
        next_map.keys().copied(),
    )
    .await;
    *ctx.subscriptions = next_map;
    *ctx.subscription_state = next_state;
    if seed_worktree_vcs_for_subscribe(
        ctx.control,
        state,
        workspace_id,
        &worktree_vcs_session_ids,
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
    spawn_worktree_vcs_refresh_for_sessions(state.clone(), worktree_vcs_session_ids);
    Ok(())
}
