use super::workspace_stream;
use super::*;

#[path = "workspace_active/send_loop.rs"]
mod send_loop;

async fn require_workspace_active_stream_access(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<(), StatusCode> {
    let exists = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some();
    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(())
}

pub(crate) async fn workspace_active_snapshot_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workspace_id = match uuid::Uuid::parse_str(&id) {
        Ok(v) => WorkspaceId(v),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if let Err(status) = require_workspace_active_stream_access(&state, workspace_id).await {
        return status.into_response();
    }
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

    let send_task = send_loop::spawn_workspace_active_send_loop(sender, workspace_id, &runtime);
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

    let (send_task, _recv_result) = super::async_util::race_join_handle(send_task, recv_loop).await;

    workspace_stream::notify_workspace_stream_shutdown(&runtime).await;
    if let Some(send_task) = send_task {
        let _ = send_task.await;
    }

    workspace_stream::release_workspace_stream(&state, &runtime).await;
}
