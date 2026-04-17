use super::workspace_stream;
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
    let labels = workspace_stream::WorkspaceStreamLabels {
        ready_queue_label: "ready",
        subscribe_resolution_log: "workspace stream subscribe resolution failed",
        replay_list_metric: "ctx_http.replay_session_events_active.list",
        replay_send_metric: Some("ctx_http.replay_session_events_active.send"),
        replay_queue_label: "replay",
        replay_failure_log: "workspace stream replay failed",
        lagged_log: "workspace stream lagged",
        event_queue_label: "event",
    };
    let Some((mut runtime, mut rx)) = workspace_stream::initialize_workspace_stream(
        &state,
        workspace_id,
        labels.ready_queue_label,
    )
    .await
    else {
        return;
    };

    let send_task = {
        let priority_control = runtime.priority_control.clone();
        let control = runtime.control.clone();
        let foreground_head_buffer = runtime.foreground_head_buffer.clone();
        let background_head_buffer = runtime.background_head_buffer.clone();
        let summary_buffer = runtime.summary_buffer.clone();
        let send_control = runtime.send_control.clone();
        let latest_snapshot_rev = runtime.latest_snapshot_rev.clone();
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
                    &summary_buffer,
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
                        NextWorkspaceStreamItem::SummaryBatch { events } => {
                            let mut send_failed = false;
                            for event in events {
                                stream_seq += 1;
                                let message = WorkspaceActiveSnapshotStreamMessage::Event {
                                    rev: stream_seq,
                                    event: Box::new(event),
                                };
                                let Ok(text) = serde_json::to_string(&message) else {
                                    send_failed = true;
                                    break;
                                };
                                if sender.send(WsMessage::Text(text)).await.is_err() {
                                    send_failed = true;
                                    break;
                                }
                            }
                            if send_failed {
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
                            &summary_buffer,
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
                    _ = summary_buffer.notify.notified() => {},
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
                                if workspace_stream::handle_workspace_stream_subscription(
                                    &state,
                                    workspace_id,
                                    message,
                                    &mut runtime,
                                    &labels,
                                )
                                .await
                                .is_err()
                                {
                                    break;
                                }
                            }
                        }
                        Some(Ok(WsMessage::Binary(bytes))) => {
                            if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                                if let Ok(message) =
                                    serde_json::from_str::<WorkspaceActiveSnapshotClientMessage>(&text)
                                {
                                    if workspace_stream::handle_workspace_stream_subscription(
                                        &state,
                                        workspace_id,
                                        message,
                                        &mut runtime,
                                        &labels,
                                    )
                                    .await
                                    .is_err()
                                    {
                                        break;
                                    }
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
                            if workspace_stream::handle_workspace_stream_lagged(
                                &state,
                                workspace_id,
                                lagged,
                                &mut runtime,
                                &labels,
                            )
                            .await
                            .is_err()
                            {
                                break;
                            }
                            continue;
                        }
                        Err(_) => break,
                    };
                    if workspace_stream::handle_workspace_stream_event(
                        &state,
                        workspace_id,
                        event,
                        &mut runtime,
                        &labels,
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                }
            }
        }
        Ok::<(), ()>(())
    };

    let (send_task, _recv_result) = crate::async_util::race_join_handle(send_task, recv_loop).await;

    workspace_stream::notify_workspace_stream_shutdown(&runtime).await;
    if let Some(send_task) = send_task {
        let _ = send_task.await;
    }

    workspace_stream::release_workspace_stream(&state, &runtime).await;
}
