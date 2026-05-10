use super::bounded::StreamQueue;
use crate::api::ws::queue::buffers::{
    HeadBatchBuffer, NextWorkspaceStreamItem, SummaryBatchBuffer,
};
use ctx_core::models::WorkspaceActiveSnapshotStreamMessage;

pub(crate) async fn take_next_workspace_stream_item(
    priority_control: &StreamQueue<WorkspaceActiveSnapshotStreamMessage>,
    control: &StreamQueue<WorkspaceActiveSnapshotStreamMessage>,
    foreground_head_buffer: &HeadBatchBuffer,
    background_head_buffer: &HeadBatchBuffer,
    summary_buffer: &SummaryBatchBuffer,
    hydrating: bool,
) -> Option<NextWorkspaceStreamItem> {
    if hydrating {
        if let Some(entry) = control.pop().await {
            return Some(NextWorkspaceStreamItem::Control(entry));
        }
        return None;
    }
    if let Some(entry) = priority_control.pop().await {
        return Some(NextWorkspaceStreamItem::Control(entry));
    }
    let (snapshot_rev, deltas) = foreground_head_buffer.take().await;
    if !deltas.is_empty() {
        return Some(NextWorkspaceStreamItem::HeadsBatch {
            snapshot_rev,
            deltas,
        });
    }
    if let Some(entry) = control.pop().await {
        return Some(NextWorkspaceStreamItem::Control(entry));
    }
    let (snapshot_rev, deltas) = background_head_buffer.take().await;
    if !deltas.is_empty() {
        return Some(NextWorkspaceStreamItem::HeadsBatch {
            snapshot_rev,
            deltas,
        });
    }
    let events = summary_buffer.take().await;
    if !events.is_empty() {
        return Some(NextWorkspaceStreamItem::SummaryBatch { events });
    }
    None
}

pub(crate) async fn workspace_stream_is_idle(
    priority_control: &StreamQueue<WorkspaceActiveSnapshotStreamMessage>,
    control: &StreamQueue<WorkspaceActiveSnapshotStreamMessage>,
    foreground_head_buffer: &HeadBatchBuffer,
    background_head_buffer: &HeadBatchBuffer,
    summary_buffer: &SummaryBatchBuffer,
) -> bool {
    priority_control.is_empty().await
        && control.is_empty().await
        && foreground_head_buffer.is_empty().await
        && background_head_buffer.is_empty().await
        && summary_buffer.is_empty().await
}
