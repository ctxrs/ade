use std::sync::Arc;

use super::*;

mod buffer;
mod metrics;
mod send_loop;
mod subscription;
#[cfg(test)]
#[path = "workspace_vcs/tests.rs"]
mod tests;

use self::buffer::VcsPendingBuffer;
use self::metrics::{record_workspace_vcs_stream_metrics, VcsStreamMetrics};
use self::send_loop::spawn_workspace_vcs_send_loop;
use self::subscription::{
    handle_workspace_vcs_client_message, release_workspace_vcs_demand, WorkspaceVcsRuntime,
};

async fn require_workspace_vcs_stream_access(
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

pub(crate) async fn workspace_vcs_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workspace_id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => WorkspaceId(value),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if let Err(status) = require_workspace_vcs_stream_access(&state, workspace_id).await {
        return status.into_response();
    }
    ws.on_upgrade(move |socket| handle_workspace_vcs_ws(socket, state, workspace_id))
}

async fn handle_workspace_vcs_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    workspace_id: WorkspaceId,
) {
    let (sender, mut receiver) = socket.split();
    let pending = Arc::new(VcsPendingBuffer::new());
    let metrics = Arc::new(VcsStreamMetrics::default());
    pending
        .push_control(WorktreeVcsStreamMessage::Ready {
            workspace_id,
            vcs_generation: 0,
        })
        .await;

    let send_task =
        spawn_workspace_vcs_send_loop(sender, Arc::clone(&pending), Arc::clone(&metrics));

    let mut runtime = WorkspaceVcsRuntime::default();
    let mut rx = state.workspaces.worktree_vcs_events.subscribe();
    let recv_loop = async {
        loop {
            tokio::select! {
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(WsMessage::Text(text))) => {
                            let Ok(message) = serde_json::from_str::<WorktreeVcsStreamClientMessage>(&text) else {
                                continue;
                            };
                            handle_workspace_vcs_client_message(
                                &state,
                                workspace_id,
                                &pending,
                                &metrics,
                                &mut runtime,
                                message,
                            )
                            .await;
                        }
                        Some(Ok(WsMessage::Binary(bytes))) => {
                            let Ok(text) = String::from_utf8(bytes.to_vec()) else {
                                continue;
                            };
                            let Ok(message) = serde_json::from_str::<WorktreeVcsStreamClientMessage>(&text) else {
                                continue;
                            };
                            handle_workspace_vcs_client_message(
                                &state,
                                workspace_id,
                                &pending,
                                &metrics,
                                &mut runtime,
                                message,
                            )
                            .await;
                        }
                        Some(Ok(WsMessage::Close(_))) => break,
                        Some(Ok(_)) => {}
                        Some(Err(_)) | None => break,
                    }
                }
                event = rx.recv() => {
                    match event {
                        Ok(snapshot) => {
                            if runtime.detail_worktree_ids.contains(&snapshot.worktree_id) {
                                subscription::queue_vcs_snapshot(
                                    &pending,
                                    &metrics,
                                    workspace_id,
                                    runtime.demand_generation,
                                    WorktreeVcsStreamTier::Details,
                                    snapshot,
                                )
                                .await;
                            } else if runtime.summary_worktree_ids.contains(&snapshot.worktree_id) {
                                subscription::queue_vcs_snapshot(
                                    &pending,
                                    &metrics,
                                    workspace_id,
                                    runtime.demand_generation,
                                    WorktreeVcsStreamTier::Summary,
                                    snapshot,
                                )
                                .await;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!(
                                target: "ctx_http.ws_vcs",
                                workspace_id = %workspace_id.0,
                                skipped,
                                "workspace vcs stream lagged; latest subscribed snapshots will be reseeded",
                            );
                            subscription::seed_current_vcs_snapshots(
                                &state,
                                workspace_id,
                                &pending,
                                &metrics,
                                runtime.demand_generation,
                                &runtime.summary_worktree_ids,
                                &runtime.detail_worktree_ids,
                            )
                            .await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    };

    recv_loop.await;
    send_task.abort();
    record_workspace_vcs_stream_metrics(&state, &metrics).await;
    release_workspace_vcs_demand(&state, &runtime).await;
}
