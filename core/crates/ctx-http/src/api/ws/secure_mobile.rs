use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use futures::StreamExt;

use ctx_core::ids::*;
use ctx_core::models::*;
use ctx_transport_runtime::mobile_e2ee;

use crate::daemon::AppState;

use super::super::{
    load_mobile_auth_context_for_profile, MobileScope, MobileSecureEnvelope,
    MobileSecureStreamQuery,
};
use super::common::{bump_latest_snapshot_rev, send_secure_ws, HEAD_BATCH_FLUSH_INTERVAL};
use super::queue::{
    take_next_workspace_stream_item, workspace_stream_is_idle, NextWorkspaceStreamItem,
};
use super::replay::with_stream_rev;
use super::workspace_stream;

async fn require_mobile_secure_stream_access(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    device_id: &str,
    provided_token: &str,
) -> Result<(), StatusCode> {
    let device_uuid = uuid::Uuid::parse_str(device_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let cfg = state
        .global_store()
        .get_mobile_access_config()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if !cfg.enabled {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let Some(mobile_auth) = load_mobile_auth_context_for_profile(state, cfg.profile_id).await?
    else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    if !mobile_auth.allows(MobileScope::WorkspaceStream) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let device = state
        .global_store()
        .get_mobile_device(MobileDeviceId(device_uuid))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if device.profile_id != cfg.profile_id {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if device
        .public_key
        .as_deref()
        .is_none_or(|key| key.trim().is_empty())
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let key = mobile_e2ee::derive_key(
        device_id,
        device.public_key.as_deref().unwrap_or_default(),
        &cfg.daemon_private_key,
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let expected_token = mobile_e2ee::derive_stream_token(&key, &workspace_id.0.to_string());
    if provided_token != expected_token {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let workspace_exists = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_some();
    if !workspace_exists {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(())
}

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

    let send_task = {
        let priority_control = runtime.priority_control.clone();
        let control = runtime.control.clone();
        let foreground_head_buffer = runtime.foreground_head_buffer.clone();
        let background_head_buffer = runtime.background_head_buffer.clone();
        let summary_buffer = runtime.summary_buffer.clone();
        let send_control = runtime.send_control.clone();
        let send_key = key.clone();
        let send_device_id = device_id.clone();
        let latest_snapshot_rev = runtime.latest_snapshot_rev.clone();
        let state = state.clone();
        let event_queue_label = labels.event_queue_label;
        tokio::spawn(async move {
            let mut sender = sender;
            let mut envelope_seq: i64 = 0;
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
                            let (enqueued_at, message) = entry.into_parts();
                            let queued_ms = enqueued_at.elapsed().as_millis();
                            let is_snapshot = matches!(
                                message,
                                WorkspaceActiveSnapshotStreamMessage::Snapshot { .. }
                            );
                            envelope_seq += 1;
                            let message = match message {
                                WorkspaceActiveSnapshotStreamMessage::ResetRequired { .. } => {
                                    message
                                }
                                other => {
                                    stream_seq += 1;
                                    with_stream_rev(other, stream_seq)
                                }
                            };
                            let send_start = Instant::now();
                            if send_secure_ws(
                                &mut sender,
                                &send_key,
                                &send_device_id,
                                envelope_seq,
                                &message,
                            )
                            .await
                            .is_err()
                            {
                                break;
                            }
                            if is_snapshot {
                                log_secure_snapshot_sent(
                                    workspace_id,
                                    queued_ms,
                                    send_start.elapsed().as_millis(),
                                    &message,
                                );
                                send_control.clear_hydrating();
                            }
                        }
                        NextWorkspaceStreamItem::HeadsBatch {
                            snapshot_rev,
                            deltas,
                        } => {
                            let latest_rev =
                                latest_snapshot_rev.load(std::sync::atomic::Ordering::Relaxed);
                            let snapshot_rev = snapshot_rev.max(latest_rev);
                            bump_latest_snapshot_rev(&latest_snapshot_rev, snapshot_rev);
                            envelope_seq += 1;
                            stream_seq += 1;
                            let message = WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
                                rev: stream_seq,
                                snapshot_rev,
                                deltas,
                            };
                            if send_secure_ws(
                                &mut sender,
                                &send_key,
                                &send_device_id,
                                envelope_seq,
                                &message,
                            )
                            .await
                            .is_err()
                            {
                                break;
                            }
                        }
                        NextWorkspaceStreamItem::SummaryBatch {
                            events,
                            vcs_coalesced_count,
                        } => {
                            workspace_stream::record_vcs_stream_coalesced(
                                &state,
                                event_queue_label,
                                vcs_coalesced_count,
                            );
                            let mut send_failed = false;
                            for event in events {
                                envelope_seq += 1;
                                stream_seq += 1;
                                let message = WorkspaceActiveSnapshotStreamMessage::Event {
                                    rev: stream_seq,
                                    event: Box::new(event),
                                };
                                if send_secure_ws(
                                    &mut sender,
                                    &send_key,
                                    &send_device_id,
                                    envelope_seq,
                                    &message,
                                )
                                .await
                                .is_err()
                                {
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
                    _ = priority_control.notify().notified() => {},
                    _ = control.notify().notified() => {},
                    _ = foreground_head_buffer.notify().notified() => {},
                    _ = background_head_buffer.notify().notified() => {},
                    _ = summary_buffer.notify().notified() => {},
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

    let (send_task, recv_result) = crate::async_util::race_join_handle(send_task, recv_loop).await;

    workspace_stream::notify_workspace_stream_shutdown(&runtime).await;
    if let Some(send_task) = send_task {
        let _ = send_task.await;
    }
    workspace_stream::release_workspace_stream(&state, &runtime).await;

    recv_result.unwrap_or(Ok(()))
}

fn log_secure_snapshot_sent(
    workspace_id: WorkspaceId,
    queued_ms: u128,
    send_ms: u128,
    message: &WorkspaceActiveSnapshotStreamMessage,
) {
    let payload_bytes = serde_json::to_vec(message)
        .map(|data| data.len())
        .unwrap_or(0);
    let (task_count, head_count) = match message {
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
        snapshot_send_ms = send_ms,
        active_tasks = task_count,
        active_heads = head_count,
        "workspace snapshot sent (secure)",
    );
}
