use super::types::{SummaryBatchPushError, SummaryBatchPushOutcome};
use super::*;

struct SummaryBatchState {
    total_len: usize,
    session_events: HashMap<SessionId, WorkspaceActiveSnapshotEvent>,
}

pub(crate) struct SummaryBatchBuffer {
    state: Mutex<SummaryBatchState>,
    notify: Notify,
    limit: usize,
}

impl SummaryBatchBuffer {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            state: Mutex::new(SummaryBatchState {
                total_len: 0,
                session_events: HashMap::new(),
            }),
            notify: Notify::new(),
            limit,
        }
    }

    pub(crate) async fn push(
        &self,
        event: WorkspaceActiveSnapshotEvent,
    ) -> Result<SummaryBatchPushOutcome, SummaryBatchPushError> {
        let mut state = self.state.lock().await;
        match &event {
            WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. } => {
                if let std::collections::hash_map::Entry::Occupied(mut entry) =
                    state.session_events.entry(delta.session_id)
                {
                    entry.insert(event);
                    self.notify.notify_one();
                    return Ok(SummaryBatchPushOutcome::Replaced);
                }
                if state.total_len >= self.limit {
                    return Err(SummaryBatchPushError::TotalLimit { limit: self.limit });
                }
                state.session_events.insert(delta.session_id, event);
            }
            _ => return Ok(SummaryBatchPushOutcome::Enqueued),
        }
        state.total_len += 1;
        self.notify.notify_one();
        Ok(SummaryBatchPushOutcome::Enqueued)
    }

    pub(crate) async fn take(&self) -> Vec<WorkspaceActiveSnapshotEvent> {
        let mut state = self.state.lock().await;
        if state.session_events.is_empty() {
            state.total_len = 0;
            return Vec::new();
        }
        let mut events = Vec::with_capacity(state.total_len);
        for (_, event) in state.session_events.drain() {
            events.push(event);
        }
        state.total_len = 0;
        events
    }

    pub(crate) async fn clear(&self) {
        let mut state = self.state.lock().await;
        state.session_events.clear();
        state.total_len = 0;
    }

    pub(crate) async fn drop_session_events_at_or_before(
        &self,
        session_id: SessionId,
        cursor: SessionReplayCursor,
    ) {
        let mut state = self.state.lock().await;
        let Some(event) = state.session_events.get(&session_id) else {
            return;
        };
        let WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. } = event else {
            return;
        };
        let Some(delta_last_event_seq) = delta.last_event_seq else {
            return;
        };
        let event_cursor = SessionReplayCursor {
            last_event_seq: delta_last_event_seq.max(0),
            projection_rev: delta.projection_rev.unwrap_or_default().max(0),
        };
        if event_cursor > cursor {
            return;
        }
        state.session_events.remove(&session_id);
        state.total_len = state.total_len.saturating_sub(1);
    }

    pub(crate) async fn is_empty(&self) -> bool {
        let state = self.state.lock().await;
        state.session_events.is_empty()
    }

    pub(crate) fn notify(&self) -> &Notify {
        &self.notify
    }

    pub(crate) fn wake(&self) {
        self.notify.notify_one();
    }
}
