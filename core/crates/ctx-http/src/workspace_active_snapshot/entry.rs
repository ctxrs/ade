use super::*;

pub(super) struct WorkspaceActiveSnapshotEntry {
    pub(super) tx: broadcast::Sender<WorkspaceActiveSnapshotEvent>,
    pub(super) snapshot_rev: i64,
    pub(super) archived_rev: i64,
    pub(super) hydrated: bool,
    pub(super) active_tasks: HashMap<TaskId, WorkspaceActiveTaskSummary>,
    pub(super) active_heads: HashMap<SessionId, SessionHeadSnapshot>,
    pub(super) session_replay: HashMap<SessionId, SessionReplayState>,
    pub(super) worktree_vcs_snapshots: HashMap<WorktreeId, WorktreeVcsSnapshot>,
}

impl WorkspaceActiveSnapshotEntry {
    pub(super) fn new() -> Self {
        let (tx, _) = broadcast::channel(512);
        Self {
            tx,
            snapshot_rev: 0,
            archived_rev: 0,
            hydrated: false,
            active_tasks: HashMap::new(),
            active_heads: HashMap::new(),
            session_replay: HashMap::new(),
            worktree_vcs_snapshots: HashMap::new(),
        }
    }

    pub(super) fn session_last_event_seq(&self, session_id: SessionId) -> i64 {
        self.session_replay
            .get(&session_id)
            .map(|state| state.last_event_seq)
            .unwrap_or(0)
    }

    pub(super) fn record_session_delta(&mut self, delta: &SessionHeadDelta) {
        let state = self.session_replay.entry(delta.session_id).or_default();
        state.record(delta);
    }

    pub(super) fn seed_session_replay(&mut self, session_id: SessionId, last_event_seq: i64) {
        let state = self.session_replay.entry(session_id).or_default();
        if last_event_seq > state.last_event_seq {
            state.last_event_seq = last_event_seq;
        }
    }

    pub(super) fn replay_session(
        &self,
        session_id: SessionId,
        after_seq: i64,
        limit: usize,
    ) -> SessionReplayResult {
        let after_seq = after_seq.max(0);
        match self.session_replay.get(&session_id) {
            Some(state) => state.replay(after_seq, limit),
            None => {
                if after_seq <= 0 {
                    SessionReplayResult::Replay {
                        deltas: Vec::new(),
                        last_sent: after_seq,
                    }
                } else {
                    let last_known_seq = self
                        .active_heads
                        .get(&session_id)
                        .map(|head| head.last_event_seq)
                        .unwrap_or(after_seq)
                        .max(after_seq);
                    SessionReplayResult::Gap {
                        last_known_seq,
                        reason: Some("missing_replay_state".to_string()),
                    }
                }
            }
        }
    }
}
