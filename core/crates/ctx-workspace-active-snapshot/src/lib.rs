use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};

use tokio::sync::{broadcast, Mutex};

use cache::{CachedSessionHead, SessionHeadCapability, SessionHeadCompleteness};
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Message, Session, SessionActivityState, SessionEvent, SessionEventType, SessionHeadDelta,
    SessionHeadSnapshot, SessionMetadata, SessionSnapshotSummary, SessionSummaryDelta, SessionTurn,
    SessionTurnToolSummary, Task, TaskDelta, TaskDeltaKind, WorkspaceActiveHeadBatch,
    WorkspaceActivePage, WorkspaceActiveSnapshot, WorkspaceActiveSnapshotEvent,
    WorkspaceActiveTaskSummary, WorkspaceTaskSummary, WorktreeBootstrapNotice, WorktreeVcsSnapshot,
};
use delta::{apply_head_delta, apply_session_summary_delta};
use entry::WorkspaceActiveSnapshotEntry;
use replay_state::{SessionReplayResult, SessionReplayState};
use trim::{compact_active_head_snapshot, is_primary_session, new_head_snapshot};

mod cache;
mod delta;
mod entry;
mod replay_state;
mod trim;

pub use replay_state::{
    is_transient_session_delta, SessionReplayCursor, WorkspaceSessionReplay,
    WorkspaceSessionReplayItem,
};
pub use trim::session_metadata_from_session;
mod stats;
pub use stats::WorkspaceActiveSnapshotStats;

pub struct WorkspaceActiveSnapshotHub {
    inner: Mutex<HashMap<WorkspaceId, WorkspaceActiveSnapshotEntry>>,
    session_heads: Mutex<HashMap<SessionId, CachedSessionHead>>,
    active_head_index: Mutex<HashMap<SessionId, WorkspaceId>>,
}

impl WorkspaceActiveSnapshotHub {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            session_heads: Mutex::new(HashMap::new()),
            active_head_index: Mutex::new(HashMap::new()),
        }
    }

    pub async fn stats(&self) -> WorkspaceActiveSnapshotStats {
        let inner = self.inner.lock().await;
        let mut active_task_count = 0;
        let mut active_head_count = 0;
        let mut session_replay_sessions = 0;
        let mut session_replay_events = 0;
        let mut session_replay_event_bytes = 0;
        let mut session_replay_event_max_bytes = 0;
        let mut workspace_stream_buffer_total = 0;
        let mut workspace_stream_buffer_max = 0;
        let mut workspace_stream_receivers_total = 0;
        let mut workspace_stream_receivers_max = 0;
        let mut active_head_bytes = 0;
        let mut active_head_max_bytes = 0;
        for entry in inner.values() {
            active_task_count += entry.active_tasks.len();
            active_head_count += entry.active_heads.len();
            session_replay_sessions += entry.session_replay.len();
            session_replay_events += entry
                .session_replay
                .values()
                .map(|state| state.event_count())
                .sum::<usize>();
            for replay in entry.session_replay.values() {
                for delta in replay.events() {
                    let bytes = serde_json::to_vec(delta).map(|buf| buf.len()).unwrap_or(0);
                    session_replay_event_bytes += bytes;
                    if bytes > session_replay_event_max_bytes {
                        session_replay_event_max_bytes = bytes;
                    }
                }
            }
            let buffer_len = entry.tx.len();
            workspace_stream_buffer_total += buffer_len;
            if buffer_len > workspace_stream_buffer_max {
                workspace_stream_buffer_max = buffer_len;
            }
            let receivers = entry.tx.receiver_count();
            workspace_stream_receivers_total += receivers;
            if receivers > workspace_stream_receivers_max {
                workspace_stream_receivers_max = receivers;
            }
            for head in entry.active_heads.values() {
                let bytes = serde_json::to_vec(head).map(|buf| buf.len()).unwrap_or(0);
                active_head_bytes += bytes;
                if bytes > active_head_max_bytes {
                    active_head_max_bytes = bytes;
                }
            }
        }
        let workspace_count = inner.len();
        drop(inner);

        let session_heads = self.session_heads.lock().await;
        let session_heads_count = session_heads.len();
        let mut session_heads_bytes = 0;
        let mut session_heads_max_bytes = 0;
        for cached in session_heads.values() {
            let bytes = serde_json::to_vec(&cached.head)
                .map(|buf| buf.len())
                .unwrap_or(0);
            session_heads_bytes += bytes;
            if bytes > session_heads_max_bytes {
                session_heads_max_bytes = bytes;
            }
        }
        drop(session_heads);
        let active_head_index_count = self.active_head_index.lock().await.len();

        WorkspaceActiveSnapshotStats {
            workspace_count,
            active_task_count,
            active_head_count,
            session_replay_sessions,
            session_replay_events,
            session_replay_event_bytes,
            session_replay_event_max_bytes,
            workspace_stream_buffer_total,
            workspace_stream_buffer_max,
            workspace_stream_receivers_total,
            workspace_stream_receivers_max,
            session_heads_count,
            session_heads_bytes,
            session_heads_max_bytes,
            active_head_index_count,
            active_head_bytes,
            active_head_max_bytes,
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

    pub async fn session_replay_cursor(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> SessionReplayCursor {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        entry.session_replay_cursor(session_id)
    }

    pub async fn replay_session_head_deltas(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        after_seq: i64,
        after_projection_rev: i64,
        limit: usize,
    ) -> SessionReplayResult {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        entry.replay_session(session_id, after_seq, after_projection_rev, limit)
    }

    pub async fn replay_session_stream(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        after_seq: i64,
        after_projection_rev: i64,
        limit: usize,
    ) -> WorkspaceSessionReplay {
        let replay = self
            .replay_session_head_deltas(
                workspace_id,
                session_id,
                after_seq,
                after_projection_rev,
                limit,
            )
            .await;
        match replay {
            SessionReplayResult::Replay { deltas, last_sent } => WorkspaceSessionReplay::Replay {
                items: deltas
                    .into_iter()
                    .map(|delta| WorkspaceSessionReplayItem::Delta(Box::new(delta)))
                    .collect(),
                last_sent,
            },
            SessionReplayResult::Gap {
                last_known_seq,
                reason,
            } => {
                if after_seq <= 0 {
                    if let Some(head) = self.get_session_head(session_id).await {
                        let last_sent = SessionReplayCursor::from_head(&head);
                        return WorkspaceSessionReplay::Replay {
                            items: vec![WorkspaceSessionReplayItem::Seed(Box::new(head))],
                            last_sent,
                        };
                    }
                    let last_sent = self.session_replay_cursor(workspace_id, session_id).await;
                    return WorkspaceSessionReplay::Replay {
                        items: Vec::new(),
                        last_sent: SessionReplayCursor {
                            last_event_seq: last_sent
                                .last_event_seq
                                .max(last_known_seq.max(after_seq)),
                            projection_rev: last_sent
                                .projection_rev
                                .max(after_projection_rev.max(0)),
                        },
                    };
                }
                let mut items = vec![WorkspaceSessionReplayItem::Gap {
                    session_id,
                    after_seq,
                    reason,
                }];
                let cursor = self.session_replay_cursor(workspace_id, session_id).await;
                let mut last_sent = SessionReplayCursor {
                    last_event_seq: cursor.last_event_seq.max(last_known_seq.max(after_seq)),
                    projection_rev: cursor.projection_rev.max(after_projection_rev.max(0)),
                };
                if let Some(head) = self.get_session_head(session_id).await {
                    last_sent = SessionReplayCursor::from_head(&head);
                    items.push(WorkspaceSessionReplayItem::Seed(Box::new(head)));
                }
                WorkspaceSessionReplay::Replay { items, last_sent }
            }
            SessionReplayResult::ResetRequired => WorkspaceSessionReplay::ResetRequired,
        }
    }

    pub async fn active_snapshot(
        &self,
        workspace_id: WorkspaceId,
        limit: i64,
    ) -> WorkspaceActiveSnapshot {
        let limit = limit.clamp(1, 200) as usize;
        let (snapshot_rev, archived_rev, total_count, mut tasks, worktree_vcs_snapshots) = {
            let guard = self.inner.lock().await;
            match guard.get(&workspace_id) {
                Some(entry) => (
                    entry.snapshot_rev,
                    entry.archived_rev,
                    entry.active_tasks.len() as i64,
                    entry.active_tasks.values().cloned().collect::<Vec<_>>(),
                    entry.worktree_vcs_snapshots.clone(),
                ),
                None => (0, 0, 0, Vec::new(), HashMap::new()),
            }
        };
        tasks.sort_by(|a, b| {
            let ord = b.task.created_at.cmp(&a.task.created_at);
            if ord == Ordering::Equal {
                b.task.id.0.cmp(&a.task.id.0)
            } else {
                ord
            }
        });
        if tasks.len() > limit {
            tasks.truncate(limit);
        }
        let mut active_worktree_ids = HashSet::new();
        for task in &tasks {
            active_worktree_ids.insert(task.primary_session.session.worktree_id);
            for summary in &task.sessions {
                active_worktree_ids.insert(summary.session.worktree_id);
            }
        }
        let mut snapshots = Vec::new();
        for worktree_id in active_worktree_ids {
            if let Some(snapshot) = worktree_vcs_snapshots.get(&worktree_id) {
                snapshots.push(snapshot.clone());
            }
        }
        WorkspaceActiveSnapshot {
            workspace_id,
            snapshot_rev,
            archived_rev,
            active: WorkspaceActivePage { tasks, total_count },
            worktree_vcs_snapshots: snapshots,
        }
    }

    pub async fn active_task_summary(
        &self,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) -> Option<WorkspaceActiveTaskSummary> {
        let guard = self.inner.lock().await;
        guard
            .get(&workspace_id)
            .and_then(|entry| entry.active_tasks.get(&task_id).cloned())
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

    pub async fn needs_hydration(&self, workspace_id: WorkspaceId) -> bool {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        !entry.hydrated
    }

    pub async fn hydrate_snapshot(
        &self,
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        archived_rev: i64,
        tasks: Vec<WorkspaceActiveTaskSummary>,
        heads: Vec<SessionHeadSnapshot>,
    ) {
        let mut tasks_by_id = HashMap::with_capacity(tasks.len());
        for task in tasks {
            tasks_by_id.insert(task.task.id, task);
        }

        let mut heads_by_id = HashMap::with_capacity(heads.len());
        for head in &heads {
            // Store a compact head for the workspace active snapshot surface to keep
            // WS payloads bounded. The full head remains available via session endpoints.
            heads_by_id.insert(head.session.id, compact_active_head_snapshot(head));
        }

        {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            if entry.hydrated {
                return;
            }
            entry.hydrated = true;
            entry.snapshot_rev = entry.snapshot_rev.max(snapshot_rev);
            entry.archived_rev = entry.archived_rev.max(archived_rev);
            entry.active_tasks = tasks_by_id;
            entry.active_heads = heads_by_id;
            let active_session_ids: HashSet<SessionId> =
                entry.active_heads.keys().cloned().collect();
            if !active_session_ids.is_empty() {
                entry
                    .session_replay
                    .retain(|session_id, _| active_session_ids.contains(session_id));
            }
            let replay_seeds: Vec<(SessionId, SessionReplayCursor)> = entry
                .active_heads
                .values()
                .map(|head| (head.session.id, SessionReplayCursor::from_head(head)))
                .collect();
            for (session_id, cursor) in replay_seeds {
                entry.seed_session_replay(session_id, cursor);
            }
        }

        if !heads.is_empty() {
            let mut session_heads = self.session_heads.lock().await;
            let mut index = self.active_head_index.lock().await;
            for head in heads {
                match session_heads.get(&head.session.id) {
                    Some(existing)
                        if existing.capability == SessionHeadCapability::ReplayCapable => {}
                    _ => {
                        session_heads.insert(
                            head.session.id,
                            CachedSessionHead {
                                workspace_id,
                                head: head.clone(),
                                completeness: SessionHeadCompleteness::Hydrated,
                                capability: SessionHeadCapability::CompactOnly,
                            },
                        );
                    }
                }
                index.insert(head.session.id, workspace_id);
            }
        }
    }

    pub async fn update_session_head(&self, head: SessionHeadSnapshot) {
        let session_id = head.session.id;
        let workspace_id = head.session.workspace_id;
        let cursor = SessionReplayCursor::from_head(&head);
        let mut session_heads = self.session_heads.lock().await;
        session_heads.insert(
            session_id,
            CachedSessionHead {
                workspace_id,
                head: head.clone(),
                completeness: SessionHeadCompleteness::Hydrated,
                capability: SessionHeadCapability::ReplayCapable,
            },
        );
        drop(session_heads);
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        entry.seed_session_replay(session_id, cursor);
        if is_primary_session(entry, session_id) {
            entry
                .active_heads
                .insert(session_id, compact_active_head_snapshot(&head));
            let mut index = self.active_head_index.lock().await;
            index.insert(session_id, workspace_id);
        }
    }

    pub async fn update_compact_session_head(&self, head: SessionHeadSnapshot) {
        let session_id = head.session.id;
        let workspace_id = head.session.workspace_id;
        let cursor = SessionReplayCursor::from_head(&head);
        let mut heads = self.session_heads.lock().await;
        let should_replace = !matches!(
            heads.get(&session_id),
            Some(existing) if existing.capability == SessionHeadCapability::ReplayCapable
        );
        if should_replace {
            heads.insert(
                session_id,
                CachedSessionHead {
                    workspace_id,
                    head: head.clone(),
                    completeness: SessionHeadCompleteness::Hydrated,
                    capability: SessionHeadCapability::CompactOnly,
                },
            );
        }
        drop(heads);
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        entry.seed_session_replay(session_id, cursor);
        if is_primary_session(entry, session_id) {
            entry
                .active_heads
                .insert(session_id, compact_active_head_snapshot(&head));
            let mut index = self.active_head_index.lock().await;
            index.insert(session_id, workspace_id);
        }
    }

    pub async fn remove_session_head(&self, session_id: SessionId) {
        let mut session_heads = self.session_heads.lock().await;
        session_heads.remove(&session_id);
    }

    pub async fn remove_session(&self, session_id: SessionId) {
        let cached_workspace_id = {
            let mut session_heads = self.session_heads.lock().await;
            session_heads
                .remove(&session_id)
                .map(|cached| cached.workspace_id)
        };
        let indexed_workspace_id = {
            let mut index = self.active_head_index.lock().await;
            index.remove(&session_id)
        };
        let mut guard = self.inner.lock().await;
        if let Some(workspace_id) = cached_workspace_id.or(indexed_workspace_id) {
            if let Some(entry) = guard.get_mut(&workspace_id) {
                entry.active_heads.remove(&session_id);
                entry.session_replay.remove(&session_id);
            }
        } else {
            for entry in guard.values_mut() {
                entry.active_heads.remove(&session_id);
                entry.session_replay.remove(&session_id);
            }
        }
    }

    pub async fn remove_workspace(&self, workspace_id: WorkspaceId) {
        let entry = {
            let mut guard = self.inner.lock().await;
            guard.remove(&workspace_id)
        };
        let mut session_ids: HashSet<SessionId> = entry
            .map(|entry| {
                entry
                    .active_heads
                    .keys()
                    .copied()
                    .chain(entry.session_replay.keys().copied())
                    .collect()
            })
            .unwrap_or_default();
        {
            let mut session_heads = self.session_heads.lock().await;
            for session_id in session_heads
                .iter()
                .filter_map(|(session_id, cached)| {
                    (cached.workspace_id == workspace_id).then_some(*session_id)
                })
                .collect::<Vec<_>>()
            {
                session_ids.insert(session_id);
            }
            for session_id in &session_ids {
                session_heads.remove(session_id);
            }
        }
        let mut index = self.active_head_index.lock().await;
        index.retain(|session_id, owner_workspace_id| {
            if *owner_workspace_id == workspace_id {
                session_ids.insert(*session_id);
                return false;
            }
            true
        });
    }

    pub async fn get_session_head(&self, session_id: SessionId) -> Option<SessionHeadSnapshot> {
        let heads = self.session_heads.lock().await;
        heads
            .get(&session_id)
            .filter(|cached| {
                cached.completeness == SessionHeadCompleteness::Hydrated
                    && cached.capability == SessionHeadCapability::ReplayCapable
            })
            .map(|cached| cached.head.clone())
    }

    pub async fn get_cached_session_head_for_request(
        &self,
        session_id: SessionId,
        include_events: bool,
        limit: u32,
    ) -> Option<SessionHeadSnapshot> {
        let requested = limit as usize;
        let mut head = {
            let heads = self.session_heads.lock().await;
            let cached = heads.get(&session_id)?;
            if cached.completeness != SessionHeadCompleteness::Hydrated {
                return None;
            }
            if include_events && cached.capability != SessionHeadCapability::ReplayCapable {
                return None;
            }
            let head = &cached.head;
            if head.turns.len() > requested {
                return None;
            }
            if head.has_more_turns && head.turns.len() < requested {
                return None;
            }
            head.clone()
        };
        if !include_events {
            head.events.clear();
            head.head_window.event_count = 0;
        }
        Some(head)
    }

    pub async fn get_cached_session_head_for_read(
        &self,
        session_id: SessionId,
    ) -> Option<SessionHeadSnapshot> {
        let heads = self.session_heads.lock().await;
        heads
            .get(&session_id)
            .filter(|cached| cached.completeness == SessionHeadCompleteness::Hydrated)
            .map(|cached| {
                let mut head = cached.head.clone();
                if cached.capability == SessionHeadCapability::CompactOnly {
                    head.events.clear();
                    head.head_window.event_count = 0;
                }
                head
            })
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

    pub async fn publish_task_delta(
        &self,
        workspace_id: WorkspaceId,
        task: Task,
        kind: TaskDeltaKind,
    ) -> bool {
        let task_id = task.id;
        let delta_task = task.clone();
        let delta_kind = kind.clone();
        let (tx, snapshot_rev, changed) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            let mut changed = false;
            match kind {
                TaskDeltaKind::Archived => {
                    if entry.active_tasks.remove(&task_id).is_some() {
                        changed = true;
                    }
                }
                TaskDeltaKind::Updated | TaskDeltaKind::Unarchived => {
                    if let Some(active_task) = entry.active_tasks.get_mut(&task_id) {
                        active_task.task = task.clone();
                        changed = true;
                    }
                }
            }
            if changed {
                entry.snapshot_rev += 1;
            }
            (entry.tx.clone(), entry.snapshot_rev, changed)
        };
        if !changed {
            return false;
        }
        let _ = tx.send(WorkspaceActiveSnapshotEvent::TaskDelta {
            workspace_id,
            snapshot_rev,
            delta: Box::new(TaskDelta {
                task: delta_task,
                kind: delta_kind,
            }),
        });
        true
    }

    pub async fn publish_session_summary_delta(
        &self,
        workspace_id: WorkspaceId,
        delta: SessionSummaryDelta,
    ) -> bool {
        let session_id = delta.session_id;
        let task_id = delta.task_id;
        let delta_for_event = delta.clone();
        let (tx, snapshot_rev, changed) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            let mut changed = false;
            if let Some(active_task) = entry.active_tasks.get_mut(&task_id) {
                if active_task.primary_session.session.id == session_id
                    && apply_session_summary_delta(&mut active_task.primary_session, &delta)
                {
                    changed = true;
                }
                if let Some(idx) = active_task
                    .sessions
                    .iter()
                    .position(|session| session.session.id == session_id)
                {
                    if apply_session_summary_delta(&mut active_task.sessions[idx], &delta) {
                        changed = true;
                    }
                }
            }
            if changed {
                entry.snapshot_rev += 1;
            }
            (entry.tx.clone(), entry.snapshot_rev, changed)
        };
        if !changed {
            return false;
        }
        let _ = tx.send(WorkspaceActiveSnapshotEvent::SessionSummaryDelta {
            workspace_id,
            snapshot_rev,
            delta: Box::new(delta_for_event),
        });
        true
    }

    pub async fn publish_session_summary(
        &self,
        workspace_id: WorkspaceId,
        summary: SessionSnapshotSummary,
    ) {
        let session_id = summary.session.id;
        let task_id = summary.session.task_id;
        let summary_for_task = summary.clone();
        let (tx, snapshot_rev, task_update) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.snapshot_rev += 1;
            let mut update = None;
            if let Some(active_task) = entry.active_tasks.get_mut(&task_id) {
                let mut changed = false;
                if active_task.primary_session.session.id == session_id {
                    active_task.primary_session = summary_for_task.clone();
                    changed = true;
                }
                if let Some(idx) = active_task
                    .sessions
                    .iter()
                    .position(|session| session.session.id == session_id)
                {
                    active_task.sessions[idx] = summary_for_task;
                    changed = true;
                }
                if changed {
                    update = Some(active_task.clone());
                }
            }
            (entry.tx.clone(), entry.snapshot_rev, update)
        };
        if let Some(task) = task_update {
            let _ = tx.send(WorkspaceActiveSnapshotEvent::ActiveTaskUpsert {
                workspace_id,
                snapshot_rev,
                task: Box::new(task),
            });
        }
        let _ = tx.send(WorkspaceActiveSnapshotEvent::SessionSummary {
            workspace_id,
            snapshot_rev,
            summary: Box::new(summary),
        });
    }

    pub async fn publish_session_head_delta(
        &self,
        workspace_id: WorkspaceId,
        session: &Session,
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
            if session.parent_session_id.is_none() {
                if let Some(head) = entry.active_heads.get_mut(&delta.session_id) {
                    apply_head_delta(head, &delta);
                } else {
                    let mut head = new_head_snapshot(session);
                    apply_head_delta(&mut head, &delta);
                    entry.active_heads.insert(delta.session_id, head);
                }
            }
            (entry.tx.clone(), entry.snapshot_rev)
        };
        {
            let mut session_heads = self.session_heads.lock().await;
            if let Some(cached) = session_heads.get_mut(&delta.session_id) {
                apply_head_delta(&mut cached.head, &delta);
            } else if session.parent_session_id.is_none() {
                let mut head = new_head_snapshot(session);
                apply_head_delta(&mut head, &delta);
                session_heads.insert(
                    delta.session_id,
                    CachedSessionHead {
                        workspace_id,
                        head,
                        completeness: SessionHeadCompleteness::DeltaOnly,
                        capability: SessionHeadCapability::CompactOnly,
                    },
                );
            }
        }
        let _ = tx.send(WorkspaceActiveSnapshotEvent::SessionHeadDelta {
            workspace_id,
            snapshot_rev,
            delta: Box::new(delta),
        });
    }

    pub async fn publish_session_gap(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
        after_seq: i64,
        reason: Option<String>,
    ) {
        let (tx, snapshot_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.snapshot_rev += 1;
            (entry.tx.clone(), entry.snapshot_rev)
        };
        let _ = tx.send(WorkspaceActiveSnapshotEvent::SessionGap {
            workspace_id,
            snapshot_rev,
            session_id,
            after_seq,
            reason,
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

    pub async fn publish_worktree_vcs_snapshot(
        &self,
        workspace_id: WorkspaceId,
        snapshot: WorktreeVcsSnapshot,
    ) {
        let (tx, snapshot_rev) = {
            let mut guard = self.inner.lock().await;
            let entry = guard
                .entry(workspace_id)
                .or_insert_with(WorkspaceActiveSnapshotEntry::new);
            entry.snapshot_rev += 1;
            entry
                .worktree_vcs_snapshots
                .insert(snapshot.worktree_id, snapshot.clone());
            (entry.tx.clone(), entry.snapshot_rev)
        };
        let _ = tx.send(WorkspaceActiveSnapshotEvent::WorktreeVcsSnapshot {
            workspace_id,
            snapshot_rev,
            snapshot: Box::new(snapshot),
        });
    }

    pub async fn drop_worktree_vcs_snapshots(&self, worktree_ids: &[WorktreeId]) {
        if worktree_ids.is_empty() {
            return;
        }
        let mut guard = self.inner.lock().await;
        for entry in guard.values_mut() {
            for worktree_id in worktree_ids {
                entry.worktree_vcs_snapshots.remove(worktree_id);
            }
        }
    }

    pub async fn publish_archived_task_upsert(
        &self,
        workspace_id: WorkspaceId,
        task: WorkspaceTaskSummary,
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

impl Default for WorkspaceActiveSnapshotHub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
