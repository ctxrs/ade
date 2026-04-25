use super::super::*;
use super::partials::try_coalesce_partial_delta;
use super::stream::StreamQueueEntry;

pub(crate) const HEAD_BATCH_TOTAL_LIMIT: usize = 1000;

struct HeadBatchState {
    snapshot_rev: i64,
    total_len: usize,
    deltas: HashMap<SessionId, Vec<SessionHeadDelta>>,
}

struct SummaryBatchState {
    total_len: usize,
    events: HashMap<SessionId, WorkspaceActiveSnapshotEvent>,
}

pub(crate) struct HeadBatchBuffer {
    state: Mutex<HeadBatchState>,
    notify: Notify,
}

pub(crate) struct SummaryBatchBuffer {
    state: Mutex<SummaryBatchState>,
    notify: Notify,
    limit: usize,
}

pub(crate) enum NextWorkspaceStreamItem {
    Control(StreamQueueEntry<WorkspaceActiveSnapshotStreamMessage>),
    HeadsBatch {
        snapshot_rev: i64,
        deltas: Vec<SessionHeadDelta>,
    },
    SummaryBatch {
        events: Vec<WorkspaceActiveSnapshotEvent>,
    },
}

#[derive(Debug)]
pub(crate) enum HeadBatchPushError {
    SessionLimit { session_id: SessionId, limit: usize },
    TotalLimit { limit: usize },
}

#[derive(Debug)]
pub(crate) enum SummaryBatchPushError {
    TotalLimit { limit: usize },
}

impl HeadBatchBuffer {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(HeadBatchState {
                snapshot_rev: 0,
                total_len: 0,
                deltas: HashMap::new(),
            }),
            notify: Notify::new(),
        }
    }

    pub(crate) async fn push(
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
            if entry.len() >= super::super::HEAD_BATCH_SESSION_LIMIT {
                return Err(HeadBatchPushError::SessionLimit {
                    session_id,
                    limit: super::super::HEAD_BATCH_SESSION_LIMIT,
                });
            }
            entry.push(delta);
        }
        state.total_len += 1;
        self.notify.notify_one();
        Ok(())
    }

    pub(crate) async fn take(&self) -> (i64, Vec<SessionHeadDelta>) {
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

    pub(crate) async fn clear(&self) {
        let mut state = self.state.lock().await;
        state.deltas.clear();
        state.total_len = 0;
        state.snapshot_rev = 0;
    }

    pub(crate) async fn is_empty(&self) -> bool {
        self.state.lock().await.deltas.is_empty()
    }

    pub(crate) fn notify(&self) -> &Notify {
        &self.notify
    }

    pub(crate) fn wake(&self) {
        self.notify.notify_one();
    }
}

impl SummaryBatchBuffer {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            state: Mutex::new(SummaryBatchState {
                total_len: 0,
                events: HashMap::new(),
            }),
            notify: Notify::new(),
            limit,
        }
    }

    pub(crate) async fn push(
        &self,
        event: WorkspaceActiveSnapshotEvent,
    ) -> Result<(), SummaryBatchPushError> {
        let session_id = match &event {
            WorkspaceActiveSnapshotEvent::SessionSummaryDelta { delta, .. } => delta.session_id,
            _ => return Ok(()),
        };
        let mut state = self.state.lock().await;
        if let std::collections::hash_map::Entry::Occupied(mut entry) =
            state.events.entry(session_id)
        {
            entry.insert(event);
            self.notify.notify_one();
            return Ok(());
        }
        if state.total_len >= self.limit {
            return Err(SummaryBatchPushError::TotalLimit { limit: self.limit });
        }
        state.events.insert(session_id, event);
        state.total_len += 1;
        self.notify.notify_one();
        Ok(())
    }

    pub(crate) async fn take(&self) -> Vec<WorkspaceActiveSnapshotEvent> {
        let mut state = self.state.lock().await;
        if state.events.is_empty() {
            state.total_len = 0;
            return Vec::new();
        }
        let mut events = Vec::with_capacity(state.total_len);
        for (_, event) in state.events.drain() {
            events.push(event);
        }
        state.total_len = 0;
        events
    }

    pub(crate) async fn clear(&self) {
        let mut state = self.state.lock().await;
        state.events.clear();
        state.total_len = 0;
    }

    pub(crate) async fn is_empty(&self) -> bool {
        self.state.lock().await.events.is_empty()
    }

    pub(crate) fn notify(&self) -> &Notify {
        &self.notify
    }

    pub(crate) fn wake(&self) {
        self.notify.notify_one();
    }
}
