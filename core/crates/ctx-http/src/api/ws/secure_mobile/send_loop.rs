use std::time::Instant;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use ctx_core::ids::WorkspaceId;
use ctx_core::models::WorkspaceActiveSnapshotStreamMessage;
use ctx_transport_runtime::mobile_e2ee;

use super::super::common::{bump_latest_snapshot_rev, send_secure_ws, HEAD_BATCH_FLUSH_INTERVAL};
use super::super::queue::{
    take_next_workspace_stream_item, workspace_stream_is_idle, NextWorkspaceStreamItem,
};
use super::super::replay::with_stream_rev;
use super::super::workspace_stream;

#[path = "send_loop/telemetry.rs"]
mod telemetry;

pub(super) fn spawn_mobile_secure_send_loop(
    sender: futures::stream::SplitSink<WebSocket, WsMessage>,
    workspace_id: WorkspaceId,
    device_id: String,
    key: mobile_e2ee::E2eeKey,
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
                            WorkspaceActiveSnapshotStreamMessage::ResetRequired { .. } => message,
                            other => {
                                stream_seq += 1;
                                with_stream_rev(other, stream_seq)
                            }
                        };
                        let send_start = Instant::now();
                        if send_secure_ws(&mut sender, &key, &device_id, envelope_seq, &message)
                            .await
                            .is_err()
                        {
                            break;
                        }
                        if is_snapshot {
                            telemetry::log_secure_snapshot_sent(
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
                        if send_secure_ws(&mut sender, &key, &device_id, envelope_seq, &message)
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    NextWorkspaceStreamItem::SummaryBatch { events } => {
                        let mut send_failed = false;
                        for event in events {
                            envelope_seq += 1;
                            stream_seq += 1;
                            let message = WorkspaceActiveSnapshotStreamMessage::Event {
                                rev: stream_seq,
                                event: Box::new(event),
                            };
                            if send_secure_ws(&mut sender, &key, &device_id, envelope_seq, &message)
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
}
