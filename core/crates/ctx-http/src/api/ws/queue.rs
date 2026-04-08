use super::*;

pub(super) const HEAD_BATCH_TOTAL_LIMIT: usize = 1000;

pub(super) struct StreamQueueEntry<T> {
    pub(super) enqueued_at: Instant,
    pub(super) message: T,
}

#[derive(Debug)]
pub(super) enum StreamQueuePushError {
    QueueFull { len: usize, limit: usize },
    QueueStale { age_ms: u64, max_age_ms: u64 },
}

pub(super) struct StreamQueue<T> {
    pending: Mutex<VecDeque<StreamQueueEntry<T>>>,
    pub(super) notify: Notify,
    limit: usize,
    max_age: Duration,
}

struct HeadBatchState {
    snapshot_rev: i64,
    total_len: usize,
    deltas: HashMap<SessionId, Vec<SessionHeadDelta>>,
}

pub(super) struct HeadBatchBuffer {
    state: Mutex<HeadBatchState>,
    pub(super) notify: Notify,
}

#[derive(Debug)]
pub(super) enum HeadBatchPushError {
    SessionLimit { session_id: SessionId, limit: usize },
    TotalLimit { limit: usize },
}

fn is_partial_event(event: &SessionEvent) -> bool {
    matches!(
        event.event_type,
        SessionEventType::AssistantChunk
            | SessionEventType::ThoughtChunk
            | SessionEventType::ContextWindowUpdate
    )
}

fn allows_partial_for_active_primary_session(
    active_task_sessions: &HashMap<TaskId, SessionId>,
    session_id: SessionId,
) -> bool {
    active_task_sessions
        .values()
        .any(|active_session_id| *active_session_id == session_id)
}

pub(super) fn filter_partial_delta_for_active_tasks(
    mut delta: SessionHeadDelta,
    active_task_sessions: &HashMap<TaskId, SessionId>,
) -> Option<SessionHeadDelta> {
    if let Some(event) = delta.event.as_ref() {
        if is_partial_event(event)
            && !allows_partial_for_active_primary_session(active_task_sessions, delta.session_id)
        {
            delta.event = None;
            if delta.turn.is_none() && delta.message.is_none() && delta.tool_summaries.is_empty() {
                return None;
            }
        }
    }
    Some(delta)
}

fn is_same_partial_type(prev: &SessionEvent, next: &SessionEvent) -> bool {
    matches!(
        (&prev.event_type, &next.event_type),
        (
            &SessionEventType::AssistantChunk,
            &SessionEventType::AssistantChunk
        ) | (
            &SessionEventType::ThoughtChunk,
            &SessionEventType::ThoughtChunk
        )
    )
}

fn extract_fragment(event: &SessionEvent) -> Option<&str> {
    event
        .payload_json
        .get("content_fragment")
        .and_then(Value::as_str)
}

pub(super) fn merge_partial_fragment(prev: &str, next: &str) -> String {
    if prev.is_empty() {
        return next.to_string();
    }
    if next.is_empty() {
        return prev.to_string();
    }
    if next.starts_with(prev) {
        return next.to_string();
    }
    if prev.ends_with(next) {
        return prev.to_string();
    }
    format!("{prev}{next}")
}

fn try_coalesce_partial_delta(entry: &mut [SessionHeadDelta], next: &SessionHeadDelta) -> bool {
    let Some(prev) = entry.last_mut() else {
        return false;
    };
    if prev.turn.is_some()
        || prev.message.is_some()
        || next.turn.is_some()
        || next.message.is_some()
    {
        return false;
    }
    let (Some(prev_event), Some(next_event)) = (prev.event.as_ref(), next.event.as_ref()) else {
        return false;
    };
    if !is_partial_event(prev_event) || !is_partial_event(next_event) {
        return false;
    }
    if !is_same_partial_type(prev_event, next_event) {
        return false;
    }
    if prev_event.turn_id != next_event.turn_id || prev_event.turn_id.is_none() {
        return false;
    }
    let (Some(prev_fragment), Some(next_fragment)) =
        (extract_fragment(prev_event), extract_fragment(next_event))
    else {
        return false;
    };
    let merged_fragment = merge_partial_fragment(prev_fragment, next_fragment);
    let mut merged_event = next_event.clone();
    match merged_event.payload_json {
        Value::Object(ref mut map) => {
            map.insert(
                "content_fragment".to_string(),
                Value::String(merged_fragment),
            );
        }
        _ => return false,
    }
    prev.event = Some(merged_event);
    prev.last_event_seq = prev.last_event_seq.max(next.last_event_seq);
    prev.projection_rev = prev.projection_rev.max(next.projection_rev);
    prev.state_rev = prev.state_rev.max(next.state_rev);
    true
}

impl HeadBatchBuffer {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(HeadBatchState {
                snapshot_rev: 0,
                total_len: 0,
                deltas: HashMap::new(),
            }),
            notify: Notify::new(),
        }
    }

    pub(super) async fn push(
        &self,
        snapshot_rev: i64,
        delta: SessionHeadDelta,
    ) -> Result<(), HeadBatchPushError> {
        let mut state = self.state.lock().await;
        let session_id = delta.session_id;
        state.snapshot_rev = state.snapshot_rev.max(snapshot_rev);
        if let Some(entry) = state.deltas.get_mut(&session_id) {
            if try_coalesce_partial_delta(entry.as_mut_slice(), &delta) {
                self.notify.notify_one();
                return Ok(());
            }
        }
        if state.total_len >= HEAD_BATCH_TOTAL_LIMIT {
            return Err(HeadBatchPushError::TotalLimit {
                limit: HEAD_BATCH_TOTAL_LIMIT,
            });
        }
        {
            let entry = state.deltas.entry(session_id).or_default();
            if entry.len() >= HEAD_BATCH_SESSION_LIMIT {
                return Err(HeadBatchPushError::SessionLimit {
                    session_id,
                    limit: HEAD_BATCH_SESSION_LIMIT,
                });
            }
            entry.push(delta);
        }
        state.total_len += 1;
        self.notify.notify_one();
        Ok(())
    }

    pub(super) async fn take(&self) -> (i64, Vec<SessionHeadDelta>) {
        let mut state = self.state.lock().await;
        if state.deltas.is_empty() {
            state.total_len = 0;
            return (state.snapshot_rev, Vec::new());
        }
        let snapshot_rev = state.snapshot_rev;
        let mut deltas = Vec::with_capacity(state.total_len);
        for (_, mut per_session) in state.deltas.drain() {
            deltas.append(&mut per_session);
        }
        state.total_len = 0;
        state.snapshot_rev = 0;
        (snapshot_rev, deltas)
    }

    pub(super) async fn clear(&self) {
        let mut state = self.state.lock().await;
        state.deltas.clear();
        state.total_len = 0;
        state.snapshot_rev = 0;
    }
}

impl<T> StreamQueue<T> {
    pub(super) fn new(limit: usize, max_age: Duration) -> Self {
        Self {
            pending: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
            limit,
            max_age,
        }
    }

    pub(super) async fn push(&self, message: T) -> Result<(), StreamQueuePushError> {
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

    pub(super) async fn clear(&self) {
        let mut guard = self.pending.lock().await;
        guard.clear();
    }

    pub(super) async fn pop(&self) -> Option<StreamQueueEntry<T>> {
        let mut guard = self.pending.lock().await;
        guard.pop_front()
    }

    pub(super) async fn is_empty(&self) -> bool {
        self.pending.lock().await.is_empty()
    }
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

pub(super) fn log_head_batch_push_error(
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

pub(super) async fn push_stream_message(
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn partial_delta(session_id: SessionId) -> SessionHeadDelta {
        SessionHeadDelta {
            session_id,
            last_event_seq: 5,
            projection_rev: 7,
            state_rev: 0,
            session: None,
            activity: None,
            event: Some(SessionEvent {
                seq: -1,
                id: SessionEventId::new(),
                session_id,
                run_id: None,
                turn_id: Some(TurnId::new()),
                event_type: SessionEventType::AssistantChunk,
                payload_json: json!({ "content_fragment": "partial" }),
                transient: true,
                created_at: Utc::now(),
            }),
            turn: None,
            message: None,
            tool_summaries: Vec::new(),
        }
    }

    #[test]
    fn filter_partial_delta_drops_non_primary_partial_only_delta() {
        let session_id = SessionId::new();
        let delta = partial_delta(session_id);
        let active_task_sessions = HashMap::new();

        assert!(filter_partial_delta_for_active_tasks(delta, &active_task_sessions).is_none());
    }

    #[test]
    fn filter_partial_delta_keeps_primary_partial_delta() {
        let session_id = SessionId::new();
        let delta = partial_delta(session_id);
        let mut active_task_sessions = HashMap::new();
        active_task_sessions.insert(TaskId::new(), session_id);

        let filtered = filter_partial_delta_for_active_tasks(delta, &active_task_sessions)
            .expect("primary session partial delta should be preserved");
        assert!(filtered.event.is_some());
    }

    #[test]
    fn filter_partial_delta_preserves_non_event_payloads() {
        let session_id = SessionId::new();
        let mut delta = partial_delta(session_id);
        let now = Utc::now();
        delta.tool_summaries.push(SessionTurnToolSummary {
            session_id,
            tool_call_id: "call-1".to_string(),
            turn_id: TurnId::new(),
            tool_kind: Some("function".to_string()),
            provider_tool_name: Some("shell".to_string()),
            title: Some("Shell".to_string()),
            subtitle: None,
            status: Some("running".to_string()),
            input_preview: None,
            output_preview: None,
            order_seq: 1,
            first_event_seq: None,
            input_truncated: None,
            input_original_bytes: None,
            output_truncated: None,
            output_original_bytes: None,
            created_at: now,
            updated_at: now,
        });
        let active_task_sessions = HashMap::new();

        let filtered = filter_partial_delta_for_active_tasks(delta, &active_task_sessions)
            .expect("non-event payload should keep delta sendable");
        assert!(filtered.event.is_none());
        assert_eq!(filtered.tool_summaries.len(), 1);
    }
}
