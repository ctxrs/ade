use super::super::partials::try_coalesce_partial_delta_tail;
use super::types::HeadBatchPushError;
use super::*;
use std::time::Instant;

pub(crate) const HEAD_BATCH_TOTAL_LIMIT: usize = 1000;
pub(crate) const BACKGROUND_HEAD_BATCH_CHUNK_LIMIT: usize = 100;

struct QueuedHeadDelta {
    enqueued_at: Instant,
    delta: SessionHeadDelta,
}

pub(crate) struct HeadBatchDrain {
    pub(crate) snapshot_rev: i64,
    pub(crate) deltas: Vec<SessionHeadDelta>,
    pub(crate) oldest_queued_ms: u128,
}

struct HeadBatchState {
    snapshot_rev: i64,
    total_len: usize,
    deltas: HashMap<SessionId, Vec<QueuedHeadDelta>>,
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
            if let Some(prev) = entry.last_mut() {
                if try_coalesce_partial_delta_tail(&mut prev.delta, &delta) {
                    self.notify.notify_one();
                    return Ok(());
                }
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
            entry.push(QueuedHeadDelta {
                enqueued_at: Instant::now(),
                delta,
            });
        }
        state.total_len += 1;
        self.notify.notify_one();
        Ok(())
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
        if state.deltas.is_empty() {
            state.total_len = 0;
            return HeadBatchDrain {
                snapshot_rev: state.snapshot_rev,
                deltas: Vec::new(),
                oldest_queued_ms: 0,
            };
        }
        if limit == 0 {
            return HeadBatchDrain {
                snapshot_rev: state.snapshot_rev,
                deltas: Vec::new(),
                oldest_queued_ms: 0,
            };
        }
        let snapshot_rev = state.snapshot_rev;
        let mut deltas = Vec::with_capacity(state.total_len.min(limit));
        let mut oldest_enqueued_at: Option<Instant> = None;
        let mut empty_sessions = Vec::new();
        let session_ids: Vec<SessionId> = state.deltas.keys().copied().collect();
        for session_id in session_ids {
            if deltas.len() >= limit {
                break;
            }
            let Some(per_session) = state.deltas.get_mut(&session_id) else {
                continue;
            };
            let take_count = (limit - deltas.len()).min(per_session.len());
            for queued in per_session.drain(..take_count) {
                if oldest_enqueued_at
                    .map(|current| queued.enqueued_at < current)
                    .unwrap_or(true)
                {
                    oldest_enqueued_at = Some(queued.enqueued_at);
                }
                deltas.push(queued.delta);
            }
            if per_session.is_empty() {
                empty_sessions.push(session_id);
            }
        }
        for session_id in empty_sessions {
            state.deltas.remove(&session_id);
        }
        state.total_len = state.total_len.saturating_sub(deltas.len());
        if state.total_len == 0 {
            state.snapshot_rev = 0;
        }
        HeadBatchDrain {
            snapshot_rev,
            deltas,
            oldest_queued_ms: oldest_enqueued_at
                .map(|enqueued_at| enqueued_at.elapsed().as_millis())
                .unwrap_or(0),
        }
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
            entry.retain(|queued| SessionReplayCursor::from_delta(&queued.delta) > cursor);
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
