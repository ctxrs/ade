use super::*;

use super::buffer::VcsPendingBuffer;
use super::metrics::{record_workspace_vcs_stream_metrics, VcsStreamMetrics};
use super::send_loop::spawn_workspace_vcs_send_loop;
use super::subscription::{
    handle_workspace_vcs_client_message, release_workspace_vcs_demand, WorkspaceVcsRuntime,
};

pub(super) async fn handle_workspace_vcs_ws(
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
    let mut rx = state.subscribe_worktree_vcs_events();
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
                                super::subscription::queue_vcs_snapshot(
                                    &pending,
                                    &metrics,
                                    workspace_id,
                                    runtime.demand_generation,
                                    WorktreeVcsStreamTier::Details,
                                    snapshot,
                                )
                                .await;
                            } else if runtime.summary_worktree_ids.contains(&snapshot.worktree_id) {
                                super::subscription::queue_vcs_snapshot(
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
                            super::subscription::seed_current_vcs_snapshots(
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
