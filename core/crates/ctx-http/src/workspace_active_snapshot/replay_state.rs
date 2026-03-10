use super::*;

pub(super) const SESSION_REPLAY_BUFFER_LIMIT: usize = 2000;

#[derive(Debug, Clone)]
pub(super) struct SessionReplayEntry {
    seq: i64,
    delta: SessionHeadDelta,
}

#[derive(Debug, Default, Clone)]
pub(super) struct SessionReplayState {
    pub(super) last_event_seq: i64,
    events: VecDeque<SessionReplayEntry>,
}

impl SessionReplayState {
    pub(super) fn record(&mut self, delta: &SessionHeadDelta) {
        let seq = delta
            .event
            .as_ref()
            .map(|event| event.seq)
            .unwrap_or(delta.last_event_seq);
        if seq >= 0 {
            self.events.push_back(SessionReplayEntry {
                seq,
                delta: delta.clone(),
            });
            while self.events.len() > SESSION_REPLAY_BUFFER_LIMIT {
                self.events.pop_front();
            }
        }
        self.last_event_seq = self.last_event_seq.max(delta.last_event_seq);
    }

    pub(super) fn replay(&self, after_seq: i64, limit: usize) -> SessionReplayResult {
        let after_seq = after_seq.max(0);
        if self.last_event_seq <= after_seq {
            return SessionReplayResult::Replay {
                deltas: Vec::new(),
                last_sent: after_seq,
            };
        }
        let Some(oldest_seq) = self.events.front().map(|entry| entry.seq) else {
            return SessionReplayResult::Gap {
                last_known_seq: self.last_event_seq,
                reason: Some("missing_replay_events".to_string()),
            };
        };
        if after_seq < oldest_seq {
            return SessionReplayResult::Gap {
                last_known_seq: self.last_event_seq,
                reason: Some("replay_buffer_overflow".to_string()),
            };
        }
        let mut deltas = Vec::new();
        for entry in self.events.iter() {
            if entry.seq > after_seq {
                deltas.push(entry.delta.clone());
            }
        }
        if deltas.is_empty() && self.last_event_seq > after_seq {
            return SessionReplayResult::Gap {
                last_known_seq: self.last_event_seq,
                reason: Some("replay_gap".to_string()),
            };
        }
        if deltas.len() > limit {
            return SessionReplayResult::Gap {
                last_known_seq: self.last_event_seq,
                reason: Some("replay_limit_exceeded".to_string()),
            };
        }
        let last_sent = deltas
            .last()
            .map(|delta| {
                delta
                    .event
                    .as_ref()
                    .map(|event| event.seq)
                    .unwrap_or(delta.last_event_seq)
            })
            .unwrap_or(after_seq);
        SessionReplayResult::Replay { deltas, last_sent }
    }

    pub(super) fn event_count(&self) -> usize {
        self.events.len()
    }

    pub(super) fn events(&self) -> impl Iterator<Item = &SessionHeadDelta> {
        self.events.iter().map(|entry| &entry.delta)
    }
}

#[derive(Debug)]
pub enum SessionReplayResult {
    Replay {
        deltas: Vec<SessionHeadDelta>,
        last_sent: i64,
    },
    Gap {
        last_known_seq: i64,
        reason: Option<String>,
    },
    ResetRequired,
}

#[derive(Debug, Clone)]
pub enum WorkspaceSessionReplayItem {
    Delta(SessionHeadDelta),
    Gap {
        session_id: SessionId,
        after_seq: i64,
        reason: Option<String>,
    },
    Seed(SessionHeadSnapshot),
}

#[derive(Debug, Clone)]
pub enum WorkspaceSessionReplay {
    Replay {
        items: Vec<WorkspaceSessionReplayItem>,
        last_sent: i64,
    },
    ResetRequired,
}
