use std::sync::Arc;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures::StreamExt;

use ctx_core::ids::*;
use ctx_core::models::*;
use ctx_transport_runtime::mobile_e2ee;

use crate::daemon::AppState;

#[path = "secure_mobile/access.rs"]
mod access;
#[path = "secure_mobile/send_loop.rs"]
mod send_loop;

use super::super::{
    load_mobile_auth_context_for_profile, MobileScope, MobileSecureEnvelope,
    MobileSecureStreamQuery,
};
use super::workspace_stream;
use access::require_mobile_secure_stream_access;

pub(in crate::api) async fn mobile_secure_workspace_stream_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<MobileSecureStreamQuery>,
) -> impl IntoResponse {
    let workspace_id = match uuid::Uuid::parse_str(&id) {
        Ok(v) => WorkspaceId(v),
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let device_id = query.device_id.trim().to_string();
    let token = query.token.trim().to_string();
    if let Err(status) =
        require_mobile_secure_stream_access(&state, workspace_id, &device_id, &token).await
    {
        return status.into_response();
    }
    ws.on_upgrade(move |socket| async move {
        if let Err(err) = handle_mobile_secure_ws(socket, state, workspace_id, device_id).await {
            tracing::warn!("secure mobile ws ended: {err:#}");
        }
    })
}

async fn handle_mobile_secure_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    workspace_id: WorkspaceId,
    device_id: String,
) -> Result<(), anyhow::Error> {
    let (sender, mut receiver) = socket.split();
    let device_uuid = uuid::Uuid::parse_str(&device_id)?;
    let cfg = state.global_store().get_mobile_access_config().await?;
    let cfg = cfg.ok_or_else(|| anyhow::anyhow!("mobile access not configured"))?;
    if !cfg.enabled {
        return Err(anyhow::anyhow!("mobile access not enabled"));
    }
    let Some(mobile_auth) = load_mobile_auth_context_for_profile(&state, cfg.profile_id)
        .await
        .map_err(|status| anyhow::anyhow!("failed to load mobile access profile: {status}"))?
    else {
        return Err(anyhow::anyhow!(
            "{}",
            MobileScope::WorkspaceStream.missing_error()
        ));
    };
    if !mobile_auth.allows(MobileScope::WorkspaceStream) {
        return Err(anyhow::anyhow!(
            "{}",
            MobileScope::WorkspaceStream.missing_error()
        ));
    }
    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await?
        .ok_or_else(|| anyhow::anyhow!("device not registered"))?;
    if device.profile_id != cfg.profile_id {
        return Err(anyhow::anyhow!("device not authorized for tunnel"));
    }
    let device_public_key = device
        .public_key
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("device missing public key"))?;
    let key = mobile_e2ee::derive_key(&device_id, device_public_key, &cfg.daemon_private_key)?;

    let labels = workspace_stream::WorkspaceStreamLabels {
        ready_queue_label: "ready_secure",
        subscribe_resolution_log: "workspace stream subscribe resolution failed (secure)",
        replay_list_metric: "ctx_http.replay_session_events_secure.list",
        replay_send_metric: None,
        replay_queue_label: "replay_secure",
        replay_failure_log: "workspace stream replay failed (secure)",
        lagged_log: "workspace stream lagged (secure)",
        event_queue_label: "event_secure",
    };
    let Some((mut runtime, mut rx)) = workspace_stream::initialize_workspace_stream(
        &state,
        workspace_id,
        labels.ready_queue_label,
    )
    .await
    else {
        return Ok(());
    };

    let send_task = send_loop::spawn_mobile_secure_send_loop(
        sender,
        workspace_id,
        device_id.clone(),
        key.clone(),
        &runtime,
    );

    let recv_loop = async {
        loop {
            tokio::select! {
                msg = receiver.next() => {
                    match msg {
                        Some(Ok(WsMessage::Text(text))) => {
                            let frame: MobileSecureEnvelope = match serde_json::from_str(&text) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };
                            let payload = mobile_e2ee::decrypt(
                                &key,
                                &device_id,
                                frame.seq,
                                &frame.nonce,
                                &frame.ciphertext,
                            )?;
                            let message: WorkspaceActiveSnapshotClientMessage =
                                serde_json::from_slice(&payload)?;
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
                        Some(Ok(WsMessage::Close(_))) => break,
                        Some(Ok(_)) => {}
                        Some(Err(_)) => break,
                        None => break,
                    }
                }
                event = rx.recv() => {
                    match event {
                        Ok(event) => {
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
                        }
                        Err(_) => break,
                    }
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    let (send_task, recv_result) = super::async_util::race_join_handle(send_task, recv_loop).await;

    workspace_stream::notify_workspace_stream_shutdown(&runtime).await;
    if let Some(send_task) = send_task {
        let _ = send_task.await;
    }
    workspace_stream::release_workspace_stream(&state, &runtime).await;

    recv_result.unwrap_or(Ok(()))
}
