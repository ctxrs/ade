use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use tokio::sync::{broadcast, Mutex};

use ctx_core::ids::{SessionId, TaskId, WorkspaceId};
use ctx_core::models::{
    SessionHeadDelta, SessionHeadSnapshot, SessionSnapshot, SessionSnapshotSummary,
    WorkspaceActiveHeadBatch, WorkspaceActivePage, WorkspaceActiveSnapshot,
    WorkspaceActiveSnapshotEvent, WorkspaceActiveTaskSummary, WorkspaceTaskSummary,
    WorktreeBootstrapNotice,
};
use ctx_store::ActiveSnapshotObserver;

pub struct WorkspaceActiveSnapshotHub {
    inner: Mutex<HashMap<WorkspaceId, WorkspaceActiveSnapshotEntry>>,
    session_heads: Mutex<HashMap<SessionId, SessionHeadSnapshot>>,
    active_head_index: Mutex<HashMap<SessionId, WorkspaceId>>,
}

const SESSION_REPLAY_BUFFER_LIMIT: usize = 2000;

#[derive(Debug, Clone)]
struct SessionReplayEntry {
    seq: i64,
    delta: SessionHeadDelta,
}

#[derive(Debug, Default, Clone)]
struct SessionReplayState {
    last_event_seq: i64,
    events: VecDeque<SessionReplayEntry>,
}

impl SessionReplayState {
    fn record(&mut self, delta: &SessionHeadDelta) {
        if let Some(event) = delta.event.as_ref() {
            if event.seq >= 0 {
                self.events.push_back(SessionReplayEntry {
                    seq: event.seq,
                    delta: delta.clone(),
                });
                while self.events.len() > SESSION_REPLAY_BUFFER_LIMIT {
                    self.events.pop_front();
                }
            }
        }
        self.last_event_seq = self.last_event_seq.max(delta.last_event_seq);
    }

    fn replay(&self, after_seq: i64, limit: usize) -> SessionReplayResult {
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
            .and_then(|delta| delta.event.as_ref().map(|event| event.seq))
            .unwrap_or(after_seq);
        SessionReplayResult::Replay { deltas, last_sent }
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

struct WorkspaceActiveSnapshotEntry {
    tx: broadcast::Sender<WorkspaceActiveSnapshotEvent>,
    snapshot_rev: i64,
    archived_rev: i64,
    active_tasks: HashMap<TaskId, WorkspaceActiveTaskSummary>,
    active_heads: HashMap<SessionId, SessionHeadSnapshot>,
    session_replay: HashMap<SessionId, SessionReplayState>,
}

impl WorkspaceActiveSnapshotEntry {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(512);
        Self {
            tx,
            snapshot_rev: 0,
            archived_rev: 0,
            active_tasks: HashMap::new(),
            active_heads: HashMap::new(),
            session_replay: HashMap::new(),
        }
    }

    fn session_last_event_seq(&self, session_id: SessionId) -> i64 {
        self.session_replay
            .get(&session_id)
            .map(|state| state.last_event_seq)
            .unwrap_or(0)
    }

    fn record_session_delta(&mut self, delta: &SessionHeadDelta) {
        let state = self.session_replay.entry(delta.session_id).or_default();
        state.record(delta);
    }

    fn seed_session_replay(&mut self, session_id: SessionId, last_event_seq: i64) {
        let state = self.session_replay.entry(session_id).or_default();
        if last_event_seq > state.last_event_seq {
            state.last_event_seq = last_event_seq;
        }
    }

    fn replay_session(
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

impl WorkspaceActiveSnapshotHub {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            session_heads: Mutex::new(HashMap::new()),
            active_head_index: Mutex::new(HashMap::new()),
        }
    }

    async fn ensure_entry(
        &self,
        workspace_id: WorkspaceId,
    ) -> broadcast::Sender<WorkspaceActiveSnapshotEvent> {
        let mut guard = self.inner.lock().await;
        guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new)
            .tx
            .clone()
    }

    pub async fn subscribe(
        &self,
        workspace_id: WorkspaceId,
    ) -> broadcast::Receiver<WorkspaceActiveSnapshotEvent> {
        self.ensure_entry(workspace_id).await.subscribe()
    }

    pub async fn snapshot_state(&self, workspace_id: WorkspaceId) -> (i64, i64) {
        let guard = self.inner.lock().await;
        guard
            .get(&workspace_id)
            .map(|entry| (entry.snapshot_rev, entry.archived_rev))
            .unwrap_or((0, 0))
    }

    pub async fn session_last_event_seq(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> i64 {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        entry.session_last_event_seq(session_id)
    }

    pub async fn replay_session_head_deltas(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        after_seq: i64,
        limit: usize,
    ) -> SessionReplayResult {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        entry.replay_session(session_id, after_seq, limit)
    }

    pub async fn active_snapshot(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> WorkspaceActiveSnapshot {
        let limit = limit.clamp(1, 200) as usize;
        let (snapshot_rev, archived_rev, total_count, mut tasks) = {
            let guard = self.inner.lock().await;
            match guard.get(&workspace_id) {
                Some(entry) => (
                    entry.snapshot_rev,
                    entry.archived_rev,
                    entry.active_tasks.len() as i64,
                    entry.active_tasks.values().cloned().collect::<Vec<_>>(),
                ),
                None => (0, 0, 0, Vec::new()),
            }
        };
        tasks.sort_by(|a, b| {
            let ord = b.sort_at.cmp(&a.sort_at);
            if ord == Ordering::Equal {
                b.task.id.0.cmp(&a.task.id.0)
            } else {
                ord
            }
        });
        if tasks.len() > limit {
            tasks.truncate(limit);
        }
        WorkspaceActiveSnapshot {
            workspace_id,
            snapshot_rev,
            archived_rev,
            active: WorkspaceActivePage { tasks, total_count },
        }
    }

    pub async fn active_heads(&self, workspace_id: WorkspaceId) -> WorkspaceActiveHeadBatch {
        let (snapshot_rev, mut heads) = {
            let guard = self.inner.lock().await;
            match guard.get(&workspace_id) {
                Some(entry) => (
                    entry.snapshot_rev,
                    entry.active_heads.values().cloned().collect::<Vec<_>>(),
                ),
                None => (0, Vec::new()),
            }
        };
        heads.sort_by(|a, b| {
            let ord = a.session.created_at.cmp(&b.session.created_at);
            if ord == Ordering::Equal {
                a.session.id.0.cmp(&b.session.id.0)
            } else {
                ord
            }
        });
        WorkspaceActiveHeadBatch {
            workspace_id,
            snapshot_rev,
            heads,
        }
    }

    pub async fn update_session_head(&self, head: SessionHeadSnapshot) {
        let mut heads = self.session_heads.lock().await;
        heads.insert(head.session.id, head);
    }

    pub async fn remove_session_head(&self, session_id: SessionId) {
        let mut heads = self.session_heads.lock().await;
        heads.remove(&session_id);
    }

    pub async fn get_session_head(&self, session_id: SessionId) -> Option<SessionHeadSnapshot> {
        let heads = self.session_heads.lock().await;
        heads.get(&session_id).cloned()
    }

    pub async fn publish_active_task_upsert(
        &self,
        workspace_id: WorkspaceId,
        task: WorkspaceActiveTaskSummary,
    ) {
        let cached = task.clone();
        let (tx, snapshot_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.snapshot_rev += 1;
            entry.active_tasks.insert(cached.task.id, cached);
            (entry.tx.clone(), entry.snapshot_rev)
        };
        let _ = tx.send(WorkspaceActiveSnapshotEvent::ActiveTaskUpsert {
            workspace_id,
            snapshot_rev,
            task: Box::new(task),
        });
    }

    pub async fn publish_active_task_delete(&self, workspace_id: WorkspaceId, task_id: TaskId) {
        let (tx, snapshot_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.snapshot_rev += 1;
            entry.active_tasks.remove(&task_id);
            (entry.tx.clone(), entry.snapshot_rev)
        };
        let _ = tx.send(WorkspaceActiveSnapshotEvent::ActiveTaskDelete {
            workspace_id,
            snapshot_rev,
            task_id,
        });
    }

    pub async fn publish_session_summary(
        &self,
        workspace_id: WorkspaceId,
        summary: SessionSnapshotSummary,
    ) {
        let (tx, snapshot_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.snapshot_rev += 1;
            (entry.tx.clone(), entry.snapshot_rev)
        };
        let _ = tx.send(WorkspaceActiveSnapshotEvent::SessionSummary {
            workspace_id,
            snapshot_rev,
            summary: Box::new(summary),
        });
    }

    pub async fn publish_session_head_delta(
        &self,
        workspace_id: WorkspaceId,
        delta: SessionHeadDelta,
        bump_snapshot: bool,
    ) {
        let (tx, snapshot_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            if bump_snapshot {
                entry.snapshot_rev += 1;
            }
            entry.record_session_delta(&delta);
            if let Some(head) = entry.active_heads.get_mut(&delta.session_id) {
                apply_head_delta(head, &delta);
            }
            (entry.tx.clone(), entry.snapshot_rev)
        };
        {
            let mut heads = self.session_heads.lock().await;
            if let Some(head) = heads.get_mut(&delta.session_id) {
                apply_head_delta(head, &delta);
            }
        }
        let _ = tx.send(WorkspaceActiveSnapshotEvent::SessionHeadDelta {
            workspace_id,
            snapshot_rev,
            delta: Box::new(delta),
        });
    }

    pub async fn publish_worktree_bootstrap(
        &self,
        workspace_id: WorkspaceId,
        notice: WorktreeBootstrapNotice,
    ) {
        let (tx, snapshot_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.snapshot_rev += 1;
            (entry.tx.clone(), entry.snapshot_rev)
        };
        let _ = tx.send(WorkspaceActiveSnapshotEvent::WorktreeBootstrap {
            workspace_id,
            snapshot_rev,
            notice,
        });
    }

    pub async fn publish_archived_task_upsert(
        &self,
        workspace_id: WorkspaceId,
        task: WorkspaceTaskSummary,
        snapshot: Option<SessionSnapshot>,
    ) {
        let (tx, archived_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.archived_rev += 1;
            (entry.tx.clone(), entry.archived_rev)
        };
        let _ = tx.send(WorkspaceActiveSnapshotEvent::ArchivedTaskUpsert {
            workspace_id,
            archived_rev,
            task: Box::new(task),
            snapshot: snapshot.map(Box::new),
        });
    }

    pub async fn publish_archived_task_delete(&self, workspace_id: WorkspaceId, task_id: TaskId) {
        let (tx, archived_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.archived_rev += 1;
            (entry.tx.clone(), entry.archived_rev)
        };
        let _ = tx.send(WorkspaceActiveSnapshotEvent::ArchivedTaskDelete {
            workspace_id,
            archived_rev,
            task_id,
        });
    }
}

#[async_trait]
impl ActiveSnapshotObserver for WorkspaceActiveSnapshotHub {
    async fn on_active_head_snapshot(&self, head: SessionHeadSnapshot) {
        let workspace_id = head.session.workspace_id;
        let session_id = head.session.id;
        let last_event_seq = head.last_event_seq;
        {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.active_heads.insert(session_id, head);
            entry.seed_session_replay(session_id, last_event_seq);
        }
        let mut index = self.active_head_index.lock().await;
        index.insert(session_id, workspace_id);
    }

    async fn on_active_head_removed(&self, session_id: SessionId) {
        let workspace_id = {
            let mut index = self.active_head_index.lock().await;
            index.remove(&session_id)
        };
        match workspace_id {
            Some(workspace_id) => {
                let mut guard = self.inner.lock().await;
                if let Some(entry) = guard.get_mut(&workspace_id) {
                    entry.active_heads.remove(&session_id);
                }
            }
            None => {
                let mut guard = self.inner.lock().await;
                for entry in guard.values_mut() {
                    if entry.active_heads.remove(&session_id).is_some() {
                        break;
                    }
                }
            }
        }
    }
}

impl Default for WorkspaceActiveSnapshotHub {
    fn default() -> Self {
        Self::new()
    }
}

fn apply_head_delta(head: &mut SessionHeadSnapshot, delta: &SessionHeadDelta) {
    let next_seq = delta
        .event
        .as_ref()
        .map(|event| event.seq)
        .unwrap_or(delta.last_event_seq);
    if next_seq > head.last_event_seq {
        head.last_event_seq = next_seq;
    }
    if delta.state_rev > head.state_rev {
        head.state_rev = delta.state_rev;
    }
}
