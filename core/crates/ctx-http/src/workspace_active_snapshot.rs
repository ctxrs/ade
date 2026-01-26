use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::{broadcast, Mutex};

use ctx_core::ids::{SessionId, TaskId, WorkspaceId};
use ctx_core::models::{
    Message, Session, SessionActivityState, SessionEvent, SessionEventType, SessionHeadDelta,
    SessionHeadSnapshot, SessionMetadata, SessionSnapshot, SessionSnapshotSummary, SessionTurn,
    SessionTurnToolSummary, WorkspaceActiveHeadBatch, WorkspaceActivePage, WorkspaceActiveSnapshot,
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
const ACTIVE_HEAD_TURN_LIMIT: usize = 5;
const ACTIVE_HEAD_MESSAGE_LIMIT: usize = 200;
const ACTIVE_HEAD_EVENT_LIMIT: usize = 200;
const ACTIVE_HEAD_BYTE_LIMIT: usize = 1_500_000;

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
    hydrated: bool,
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
            hydrated: false,
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
        WorkspaceActiveSnapshot {
            workspace_id,
            snapshot_rev,
            archived_rev,
            active: WorkspaceActivePage { tasks, total_count },
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
        for head in &mut heads {
            strip_active_head_events(head);
        }
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
            heads_by_id.insert(head.session.id, head.clone());
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
            let replay_seeds: Vec<(SessionId, i64)> = entry
                .active_heads
                .values()
                .map(|head| (head.session.id, head.last_event_seq))
                .collect();
            for (session_id, last_event_seq) in replay_seeds {
                entry.seed_session_replay(session_id, last_event_seq);
            }
        }

        if !heads.is_empty() {
            let mut session_heads = self.session_heads.lock().await;
            let mut index = self.active_head_index.lock().await;
            for head in heads {
                session_heads.insert(head.session.id, head.clone());
                index.insert(head.session.id, workspace_id);
            }
        }
    }

    pub async fn update_session_head(&self, head: SessionHeadSnapshot) {
        let session_id = head.session.id;
        let workspace_id = head.session.workspace_id;
        let last_event_seq = head.last_event_seq;
        let mut heads = self.session_heads.lock().await;
        heads.insert(session_id, head.clone());
        drop(heads);
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(workspace_id)
            .or_insert_with(WorkspaceActiveSnapshotEntry::new);
        if is_primary_session(entry, session_id) {
            entry.active_heads.insert(session_id, head);
            entry.seed_session_replay(session_id, last_event_seq);
            let mut index = self.active_head_index.lock().await;
            index.insert(session_id, workspace_id);
        }
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
            let mut heads = self.session_heads.lock().await;
            if let Some(head) = heads.get_mut(&delta.session_id) {
                apply_head_delta(head, &delta);
            } else if session.parent_session_id.is_none() {
                let mut head = new_head_snapshot(session);
                apply_head_delta(&mut head, &delta);
                heads.insert(delta.session_id, head);
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

#[derive(Serialize)]
struct SessionHeadWindowPayload<'a> {
    turns: &'a [SessionTurn],
    tool_summaries: &'a [SessionTurnToolSummary],
    events: &'a [SessionEvent],
    messages: &'a [Message],
}

fn session_metadata_from_session(session: &Session) -> SessionMetadata {
    SessionMetadata {
        id: session.id,
        task_id: session.task_id,
        workspace_id: session.workspace_id,
        worktree_id: session.worktree_id,
        parent_session_id: session.parent_session_id,
        relationship: session.relationship.clone(),
        provider_id: session.provider_id.clone(),
        model_id: session.model_id.clone(),
        title: session.title.clone(),
        agent_role: session.agent_role.clone(),
        status: session.status.clone(),
        provider_session_ref: session.provider_session_ref.clone(),
        created_at: session.created_at,
        updated_at: session.updated_at,
    }
}

fn new_head_window() -> ctx_core::models::SessionHeadWindow {
    ctx_core::models::SessionHeadWindow {
        turn_limit: ACTIVE_HEAD_TURN_LIMIT as i64,
        message_limit: ACTIVE_HEAD_MESSAGE_LIMIT as i64,
        event_limit: ACTIVE_HEAD_EVENT_LIMIT as i64,
        byte_limit: ACTIVE_HEAD_BYTE_LIMIT as i64,
        turn_count: 0,
        message_count: 0,
        event_count: 0,
        bytes: 0,
        truncated: false,
    }
}

fn new_head_snapshot(session: &Session) -> SessionHeadSnapshot {
    SessionHeadSnapshot {
        session: session_metadata_from_session(session),
        turns: Vec::new(),
        tool_summaries: Vec::new(),
        events: Vec::new(),
        messages: Vec::new(),
        last_event_seq: 0,
        state_rev: 0,
        activity: SessionActivityState::default(),
        has_more_turns: false,
        history_cursor: None,
        has_more_history: false,
        summary_checkpoint: None,
        head_window: new_head_window(),
    }
}

fn is_primary_session(entry: &WorkspaceActiveSnapshotEntry, session_id: SessionId) -> bool {
    entry.active_tasks.values().any(|summary| {
        let primary_id = summary
            .task
            .primary_session_id
            .unwrap_or(summary.primary_session.session.id);
        primary_id == session_id
    })
}

fn is_partial_event(event: &SessionEvent) -> bool {
    matches!(
        event.event_type,
        SessionEventType::AssistantChunk | SessionEventType::ThoughtChunk
    )
}

fn should_include_event(event: &SessionEvent) -> bool {
    if event.seq < 0 {
        return false;
    }
    !is_partial_event(event)
}

fn compare_turn_order(a: &SessionTurn, b: &SessionTurn) -> Ordering {
    match (a.start_seq, b.start_seq) {
        (Some(sa), Some(sb)) if sa != sb => sa.cmp(&sb),
        _ => a.started_at.cmp(&b.started_at),
    }
}

fn upsert_turn(turns: &mut Vec<SessionTurn>, next: &SessionTurn) {
    if let Some(pos) = turns.iter().position(|turn| turn.turn_id == next.turn_id) {
        turns[pos] = next.clone();
    } else {
        turns.push(next.clone());
    }
    turns.sort_by(compare_turn_order);
}

fn compare_message_order(a: &Message, b: &Message) -> Ordering {
    let created = a.created_at.cmp(&b.created_at);
    if created != Ordering::Equal {
        return created;
    }
    match (a.turn_sequence, b.turn_sequence) {
        (Some(sa), Some(sb)) if sa != sb => sa.cmp(&sb),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        _ => a.id.0.cmp(&b.id.0),
    }
}

fn upsert_message(messages: &mut Vec<Message>, next: &Message) {
    if let Some(pos) = messages.iter().position(|msg| msg.id == next.id) {
        messages[pos] = next.clone();
    } else {
        messages.push(next.clone());
    }
    messages.sort_by(compare_message_order);
}

fn upsert_event(events: &mut Vec<SessionEvent>, next: &SessionEvent) {
    if let Some(pos) = events.iter().position(|event| event.seq == next.seq) {
        events[pos] = next.clone();
    } else {
        events.push(next.clone());
    }
    events.sort_by(|a, b| a.seq.cmp(&b.seq));
}

fn head_window_bytes(
    turns: &[SessionTurn],
    tool_summaries: &[SessionTurnToolSummary],
    events: &[SessionEvent],
    messages: &[Message],
) -> usize {
    let payload = SessionHeadWindowPayload {
        turns,
        tool_summaries,
        events,
        messages,
    };
    serde_json::to_vec(&payload)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn retain_messages_for_turns(messages: &mut Vec<Message>, turns: &[SessionTurn]) {
    if turns.is_empty() {
        messages.clear();
        return;
    }
    let mut allowed = HashSet::new();
    for turn in turns {
        allowed.insert(turn.turn_id);
    }
    messages.retain(|msg| match msg.turn_id {
        Some(turn_id) => allowed.contains(&turn_id),
        None => true,
    });
}

fn retain_tool_summaries_for_turns(
    tool_summaries: &mut Vec<SessionTurnToolSummary>,
    turns: &[SessionTurn],
) {
    if turns.is_empty() {
        tool_summaries.clear();
        return;
    }
    let mut allowed = HashSet::new();
    for turn in turns {
        allowed.insert(turn.turn_id);
    }
    tool_summaries.retain(|tool| allowed.contains(&tool.turn_id));
}

fn strip_snapshot_partials(turns: &mut [SessionTurn], events: &mut Vec<SessionEvent>) {
    for turn in turns.iter_mut() {
        turn.assistant_partial = None;
        turn.thought_partial = None;
    }
    events.retain(should_include_event);
}

fn strip_active_head_events(head: &mut SessionHeadSnapshot) {
    if head.events.is_empty() {
        return;
    }
    head.events.clear();
    let bytes = head_window_bytes(
        &head.turns,
        &head.tool_summaries,
        &head.events,
        &head.messages,
    );
    head.head_window.event_limit = 0;
    head.head_window.event_count = 0;
    head.head_window.bytes = bytes as i64;
}

fn trim_head_window(head: &mut SessionHeadSnapshot) {
    let turn_limit = if head.head_window.turn_limit > 0 {
        head.head_window.turn_limit as usize
    } else {
        ACTIVE_HEAD_TURN_LIMIT
    };
    let message_limit = if head.head_window.message_limit > 0 {
        head.head_window.message_limit as usize
    } else {
        ACTIVE_HEAD_MESSAGE_LIMIT
    };
    let event_limit = if head.head_window.event_limit > 0 {
        head.head_window.event_limit as usize
    } else {
        ACTIVE_HEAD_EVENT_LIMIT
    };
    let byte_limit = if head.head_window.byte_limit > 0 {
        head.head_window.byte_limit as usize
    } else {
        ACTIVE_HEAD_BYTE_LIMIT
    };

    let mut truncated = false;
    while head.turns.len() > turn_limit {
        head.turns.remove(0);
        truncated = true;
        head.has_more_turns = true;
    }
    retain_messages_for_turns(&mut head.messages, &head.turns);
    retain_tool_summaries_for_turns(&mut head.tool_summaries, &head.turns);

    while head.messages.len() > message_limit && !head.turns.is_empty() {
        head.turns.remove(0);
        truncated = true;
        head.has_more_turns = true;
        retain_messages_for_turns(&mut head.messages, &head.turns);
        retain_tool_summaries_for_turns(&mut head.tool_summaries, &head.turns);
    }

    if head.events.len() > event_limit {
        let drop = head.events.len() - event_limit;
        head.events.drain(0..drop);
        truncated = true;
    }

    loop {
        let bytes = head_window_bytes(
            &head.turns,
            &head.tool_summaries,
            &head.events,
            &head.messages,
        );
        if bytes <= byte_limit || (head.turns.is_empty() && head.events.is_empty()) {
            break;
        }
        if !head.turns.is_empty() {
            head.turns.remove(0);
            truncated = true;
            head.has_more_turns = true;
            retain_messages_for_turns(&mut head.messages, &head.turns);
            retain_tool_summaries_for_turns(&mut head.tool_summaries, &head.turns);
            continue;
        }
        if !head.events.is_empty() {
            head.events.remove(0);
            truncated = true;
            continue;
        }
        break;
    }

    let bytes = head_window_bytes(
        &head.turns,
        &head.tool_summaries,
        &head.events,
        &head.messages,
    );
    head.head_window = ctx_core::models::SessionHeadWindow {
        turn_limit: turn_limit as i64,
        message_limit: message_limit as i64,
        event_limit: event_limit as i64,
        byte_limit: byte_limit as i64,
        turn_count: head.turns.len() as i64,
        message_count: head.messages.len() as i64,
        event_count: head.events.len() as i64,
        bytes: bytes as i64,
        truncated,
    };
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

    let mut changed = false;
    if let Some(turn) = delta.turn.as_ref() {
        upsert_turn(&mut head.turns, turn);
        changed = true;
    }
    if let Some(message) = delta.message.as_ref() {
        upsert_message(&mut head.messages, message);
        changed = true;
    }
    if let Some(event) = delta.event.as_ref() {
        if should_include_event(event) {
            upsert_event(&mut head.events, event);
            changed = true;
        }
    }
    if changed {
        strip_snapshot_partials(&mut head.turns, &mut head.events);
        trim_head_window(head);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_records_delta_without_event() {
        let session_id = SessionId(uuid::Uuid::nil());
        let delta = SessionHeadDelta {
            session_id,
            last_event_seq: 5,
            state_rev: 0,
            event: None,
            turn: None,
            message: None,
        };
        let delta_next = SessionHeadDelta {
            session_id,
            last_event_seq: 6,
            state_rev: 0,
            event: None,
            turn: None,
            message: None,
        };
        let mut state = SessionReplayState::default();
        state.record(&delta);
        state.record(&delta_next);

        match state.replay(5, 10) {
            SessionReplayResult::Replay { deltas, last_sent } => {
                assert_eq!(last_sent, 6);
                assert_eq!(deltas.len(), 1);
                assert_eq!(deltas[0].last_event_seq, 6);
            }
            other => panic!("expected replay, got {other:?}"),
        }
    }
}
