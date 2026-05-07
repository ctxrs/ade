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
    session_events: HashMap<SessionId, WorkspaceActiveSnapshotEvent>,
    worktree_vcs_events: HashMap<WorktreeId, WorkspaceActiveSnapshotEvent>,
    vcs_coalesced_count: u64,
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
        vcs_coalesced_count: u64,
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

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SummaryBatchPushOutcome {
    Enqueued,
    Replaced,
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

    pub(crate) async fn drop_session_deltas_at_or_before(
        &self,
        session_id: SessionId,
        cursor: SessionReplayCursor,
    ) {
        let mut state = self.state.lock().await;
        let mut remove_entry = false;
        let removed = if let Some(entry) = state.deltas.get_mut(&session_id) {
            let before = entry.len();
            entry.retain(|delta| SessionReplayCursor::from_delta(delta) > cursor);
            remove_entry = entry.is_empty();
            before.saturating_sub(entry.len())
        } else {
            0
        };
        if remove_entry {
            state.deltas.remove(&session_id);
        }
        state.total_len = state.total_len.saturating_sub(removed);
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
                session_events: HashMap::new(),
                worktree_vcs_events: HashMap::new(),
                vcs_coalesced_count: 0,
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
            WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot { snapshot, .. } => {
                let worktree_id = snapshot.worktree_id;
                if let std::collections::hash_map::Entry::Occupied(mut entry) =
                    state.worktree_vcs_events.entry(worktree_id)
                {
                    entry.insert(event);
                    state.vcs_coalesced_count = state.vcs_coalesced_count.saturating_add(1);
                    self.notify.notify_one();
                    return Ok(SummaryBatchPushOutcome::Replaced);
                }
                if state.total_len >= self.limit {
                    return Err(SummaryBatchPushError::TotalLimit { limit: self.limit });
                }
                state.worktree_vcs_events.insert(worktree_id, event);
            }
            _ => return Ok(SummaryBatchPushOutcome::Enqueued),
        }
        state.total_len += 1;
        self.notify.notify_one();
        Ok(SummaryBatchPushOutcome::Enqueued)
    }

    pub(crate) async fn take(&self) -> (Vec<WorkspaceActiveSnapshotEvent>, u64) {
        let mut state = self.state.lock().await;
        let vcs_coalesced_count = state.vcs_coalesced_count;
        state.vcs_coalesced_count = 0;
        if state.session_events.is_empty() && state.worktree_vcs_events.is_empty() {
            state.total_len = 0;
            return (Vec::new(), vcs_coalesced_count);
        }
        let mut events = Vec::with_capacity(state.total_len);
        for (_, event) in state.session_events.drain() {
            events.push(event);
        }
        for (_, event) in state.worktree_vcs_events.drain() {
            events.push(event);
        }
        state.total_len = 0;
        (events, vcs_coalesced_count)
    }

    pub(crate) async fn clear(&self) {
        let mut state = self.state.lock().await;
        state.session_events.clear();
        state.worktree_vcs_events.clear();
        state.total_len = 0;
        state.vcs_coalesced_count = 0;
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
        state.session_events.is_empty() && state.worktree_vcs_events.is_empty()
    }

    pub(crate) fn notify(&self) -> &Notify {
        &self.notify
    }

    pub(crate) fn wake(&self) {
        self.notify.notify_one();
    }
}
