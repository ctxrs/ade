use std::sync::atomic::Ordering;
use std::time::Instant;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures::SinkExt;

use super::super::common::{bump_latest_snapshot_rev, HEAD_BATCH_FLUSH_INTERVAL};
use super::super::queue::{
    take_next_workspace_stream_item, workspace_stream_is_idle, NextWorkspaceStreamItem,
};
use super::super::replay::with_stream_rev;
use super::super::workspace_stream;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::WorkspaceActiveSnapshotStreamMessage;

pub(super) fn spawn_workspace_active_send_loop(
    sender: futures::stream::SplitSink<WebSocket, WsMessage>,
    workspace_id: WorkspaceId,
    runtime: &workspace_stream::WorkspaceStreamRuntime,
) -> tokio::task::JoinHandle<()> {
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
                        let (enqueued_at, message) = entry.into_parts();
                        let queued_ms = enqueued_at.elapsed().as_millis();
                        let is_snapshot = matches!(
                            message,
                            WorkspaceActiveSnapshotStreamMessage::Snapshot { .. }
                        );
                        let message = match message {
                            WorkspaceActiveSnapshotStreamMessage::ResetRequired { .. } => message,
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
                            log_workspace_snapshot_sent(
                                workspace_id,
                                payload_bytes,
                                queued_ms,
                                serialize_start.elapsed().as_millis(),
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
                _ = priority_control.notify().notified() => {},
                _ = control.notify().notified() => {},
                _ = foreground_head_buffer.notify().notified() => {},
                _ = background_head_buffer.notify().notified() => {},
                _ = summary_buffer.notify().notified() => {},
                _ = tick.tick() => {},
            }
        }
    })
}

fn log_workspace_snapshot_sent(
    workspace_id: WorkspaceId,
    payload_bytes: usize,
    queued_ms: u128,
    encode_ms: u128,
    send_ms: u128,
    message: &WorkspaceActiveSnapshotStreamMessage,
) {
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
        snapshot_encode_ms = encode_ms,
        snapshot_send_ms = send_ms,
        active_tasks = task_count,
        active_heads = head_count,
        "workspace snapshot sent",
    );
}
