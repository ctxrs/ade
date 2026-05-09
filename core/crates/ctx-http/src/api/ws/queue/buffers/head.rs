use super::super::partials::try_coalesce_partial_delta;
use super::types::HeadBatchPushError;
use super::*;

pub(crate) const HEAD_BATCH_TOTAL_LIMIT: usize = 1000;

struct HeadBatchState {
    snapshot_rev: i64,
    total_len: usize,
    deltas: HashMap<SessionId, Vec<SessionHeadDelta>>,
}

pub(crate) struct HeadBatchBuffer {
    state: Mutex<HeadBatchState>,
    notify: Notify,
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
            if entry.len() >= super::super::super::HEAD_BATCH_SESSION_LIMIT {
                return Err(HeadBatchPushError::SessionLimit {
                    session_id,
                    limit: super::super::super::HEAD_BATCH_SESSION_LIMIT,
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
