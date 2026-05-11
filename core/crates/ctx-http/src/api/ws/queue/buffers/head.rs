use super::types::HeadBatchPushError;
use super::*;

mod state;

use state::HeadBatchState;

pub(crate) const HEAD_BATCH_TOTAL_LIMIT: usize = 1000;
pub(crate) const BACKGROUND_HEAD_BATCH_CHUNK_LIMIT: usize = 100;

pub(crate) struct HeadBatchDrain {
    pub(crate) snapshot_rev: i64,
    pub(crate) deltas: Vec<SessionHeadDelta>,
    pub(crate) oldest_queued_ms: u128,
}

pub(crate) struct HeadBatchBuffer {
    state: Mutex<HeadBatchState>,
    notify: Notify,
}

impl HeadBatchBuffer {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(HeadBatchState::new()),
            notify: Notify::new(),
        }
    }

    pub(crate) async fn push(
        &self,
        snapshot_rev: i64,
        delta: SessionHeadDelta,
    ) -> Result<(), HeadBatchPushError> {
        let mut state = self.state.lock().await;
        let result = state.push(snapshot_rev, delta);
        if result.is_ok() {
            self.notify.notify_one();
        }
        result
    }

    #[cfg(test)]
    pub(crate) async fn take(&self) -> (i64, Vec<SessionHeadDelta>) {
        let drained = self.take_with_meta().await;
        (drained.snapshot_rev, drained.deltas)
    }

    pub(crate) async fn take_with_meta(&self) -> HeadBatchDrain {
        self.take_chunk_with_meta(usize::MAX).await
    }

    pub(crate) async fn take_chunk_with_meta(&self, limit: usize) -> HeadBatchDrain {
        let mut state = self.state.lock().await;
        state.take_chunk_with_meta(limit)
    }

    pub(crate) async fn clear(&self) {
        let mut state = self.state.lock().await;
        state.clear();
    }

    pub(crate) async fn drop_session_deltas_at_or_before(
        &self,
        session_id: SessionId,
        cursor: SessionReplayCursor,
    ) {
        let mut state = self.state.lock().await;
        state.drop_session_deltas_at_or_before(session_id, cursor);
    }

    pub(crate) async fn is_empty(&self) -> bool {
        self.state.lock().await.is_empty()
    }

    pub(crate) fn notify(&self) -> &Notify {
        &self.notify
    }

    pub(crate) fn wake(&self) {
        self.notify.notify_one();
    }
}
