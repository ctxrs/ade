use super::super::*;
use super::buffers::{
    HeadBatchBuffer, HeadBatchPushError, NextWorkspaceStreamItem, SummaryBatchBuffer,
    SummaryBatchPushError,
};
use super::Duration;

pub(crate) struct StreamQueueEntry<T> {
    enqueued_at: Instant,
    message: T,
}

impl<T> StreamQueueEntry<T> {
    pub(crate) fn into_parts(self) -> (Instant, T) {
        (self.enqueued_at, self.message)
    }
}

#[derive(Debug)]
pub(crate) enum StreamQueuePushError {
    QueueFull { len: usize, limit: usize },
    QueueStale { age_ms: u64, max_age_ms: u64 },
}

pub(crate) struct StreamQueue<T> {
    pending: Mutex<VecDeque<StreamQueueEntry<T>>>,
    notify: Notify,
    limit: usize,
    max_age: Duration,
}

impl<T> StreamQueue<T> {
    pub(crate) fn new(limit: usize, max_age: Duration) -> Self {
        Self {
            pending: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
            limit,
            max_age,
        }
    }

    pub(crate) async fn push(&self, message: T) -> Result<(), StreamQueuePushError> {
        let now = Instant::now();
        let mut guard = self.pending.lock().await;
        let len = guard.len();
        if len >= self.limit {
            return Err(StreamQueuePushError::QueueFull {
                len,
                limit: self.limit,
            });
        }
        if let Some(front) = guard.front() {
            let age = now.duration_since(front.enqueued_at);
            if age >= self.max_age {
                return Err(StreamQueuePushError::QueueStale {
                    age_ms: age.as_millis() as u64,
                    max_age_ms: self.max_age.as_millis() as u64,
                });
            }
        }
        guard.push_back(StreamQueueEntry {
            enqueued_at: now,
            message,
        });
        self.notify.notify_one();
        Ok(())
    }

    pub(crate) async fn clear(&self) {
        let mut guard = self.pending.lock().await;
        guard.clear();
    }

    pub(crate) async fn pop(&self) -> Option<StreamQueueEntry<T>> {
        let mut guard = self.pending.lock().await;
        guard.pop_front()
    }

    pub(crate) async fn is_empty(&self) -> bool {
        self.pending.lock().await.is_empty()
    }

    pub(crate) fn notify(&self) -> &Notify {
        &self.notify
    }

    pub(crate) fn wake(&self) {
        self.notify.notify_one();
    }
}

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

fn log_stream_queue_push_error(
    context: &'static str,
    workspace_id: WorkspaceId,
    session_id: Option<SessionId>,
    err: &StreamQueuePushError,
) {
    match err {
        StreamQueuePushError::QueueFull { len, limit } => {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                session_id = ?session_id,
                queue_len = *len,
                queue_limit = *limit,
                "workspace stream queue full ({context})",
            );
        }
        StreamQueuePushError::QueueStale { age_ms, max_age_ms } => {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                session_id = ?session_id,
                oldest_age_ms = *age_ms,
                max_age_ms = *max_age_ms,
                "workspace stream queue stale ({context})",
            );
        }
    }
}

pub(crate) fn log_head_batch_push_error(
    context: &'static str,
    workspace_id: WorkspaceId,
    err: &HeadBatchPushError,
) {
    match err {
        HeadBatchPushError::SessionLimit { session_id, limit } => {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                session_id = %session_id.0,
                limit = *limit,
                "workspace head batch session limit exceeded ({context})",
            );
        }
        HeadBatchPushError::TotalLimit { limit } => {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                limit = *limit,
                "workspace head batch total limit exceeded ({context})",
            );
        }
    }
}

pub(crate) fn log_summary_batch_push_error(
    context: &'static str,
    workspace_id: WorkspaceId,
    err: &SummaryBatchPushError,
) {
    match err {
        SummaryBatchPushError::TotalLimit { limit } => {
            tracing::error!(
                target: "ctx_http.ws_active_snapshot",
                workspace_id = %workspace_id.0,
                limit = *limit,
                "workspace summary batch total limit exceeded ({context})",
            );
        }
    }
}

pub(crate) async fn push_stream_message(
    pending: &StreamQueue<WorkspaceActiveSnapshotStreamMessage>,
    workspace_id: WorkspaceId,
    session_id: Option<SessionId>,
    context: &'static str,
    message: WorkspaceActiveSnapshotStreamMessage,
) -> Result<(), ()> {
    match pending.push(message).await {
        Ok(()) => Ok(()),
        Err(err) => {
            log_stream_queue_push_error(context, workspace_id, session_id, &err);
            Err(())
        }
    }
}
