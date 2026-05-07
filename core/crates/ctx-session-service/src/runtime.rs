use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc, watch, Mutex};

use ctx_core::ids::{SessionId, TaskId, TurnId, WorkspaceId};
use ctx_core::models::{
    Session, SessionEvent, SessionEventType, SessionHeadDelta, SessionHeadSnapshot,
    SessionSummaryDelta, SessionTurn, SessionTurnToolSummary,
};
use ctx_session_tools::order_seq::OrderSeqState;
use ctx_store::Store;

use crate::head_projection::{
    activity_from_turn, build_session_summary_delta, derive_message_preview,
    derive_summary_activity, event_context_window, is_session_gap_notice, message_from_event,
    patch_turn_from_event, recompute_turn_tool_counts, resolve_projection_rev_for_stream_delta,
    session_metadata_from_session, should_include_session_metadata_in_head_delta,
    should_refresh_turn_from_store, turn_from_event,
};

const DEFAULT_PROVIDER_INACTIVITY_TIMEOUT_SECS: u64 = 30 * 60;
const TASK_DELTA_REFRESH_DEBOUNCE_MS: u64 = 250;

pub struct SessionRuntime<SchedulerCommand> {
    pub session_head_cache:
        Mutex<HashMap<SessionId, TimedEntry<HashMap<SessionHeadCacheKey, SessionHeadSnapshot>>>>,
    pub schedulers: Mutex<HashMap<SessionId, TimedEntry<mpsc::Sender<SchedulerCommand>>>>,
    pub provider_inactivity_timeout: Mutex<Duration>,
    pub broadcasters: Mutex<HashMap<SessionId, TimedEntry<broadcast::Sender<SessionEvent>>>>,
    pub session_event_heads: Mutex<HashMap<SessionId, TimedEntry<watch::Sender<i64>>>>,
    pub order_seq_states: Mutex<HashMap<SessionId, TimedEntry<Arc<Mutex<OrderSeqState>>>>>,
    pub active_task_refreshes: Arc<Mutex<HashMap<TaskId, ActiveTaskRefreshEntry>>>,
    pub task_session_creation_locks:
        Mutex<HashMap<TaskId, std::sync::Weak<tokio::sync::Mutex<()>>>>,
    pub running_sessions: Arc<Mutex<HashSet<SessionId>>>,
    pub session_pins: Mutex<HashMap<SessionId, SessionPinState>>,
    pub session_meta_cache: Mutex<HashMap<SessionId, TimedEntry<Session>>>,
}

impl<SchedulerCommand> SessionRuntime<SchedulerCommand> {
    pub fn new(provider_inactivity_timeout: Duration) -> Self {
        Self {
            session_head_cache: Mutex::new(HashMap::new()),
            schedulers: Mutex::new(HashMap::new()),
            provider_inactivity_timeout: Mutex::new(provider_inactivity_timeout),
            broadcasters: Mutex::new(HashMap::new()),
            session_event_heads: Mutex::new(HashMap::new()),
            order_seq_states: Mutex::new(HashMap::new()),
            active_task_refreshes: Arc::new(Mutex::new(HashMap::new())),
            task_session_creation_locks: Mutex::new(HashMap::new()),
            running_sessions: Arc::new(Mutex::new(HashSet::new())),
            session_pins: Mutex::new(HashMap::new()),
            session_meta_cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn new_from_env() -> Self {
        Self::new(provider_inactivity_timeout_from_env())
    }

    pub async fn task_session_creation_lock(&self, task_id: TaskId) -> Arc<Mutex<()>> {
        let mut locks = self.task_session_creation_locks.lock().await;
        locks.retain(|_, weak| weak.upgrade().is_some());
        match locks.get(&task_id).and_then(std::sync::Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(task_id, Arc::downgrade(&lock));
                lock
            }
        }
    }

    pub async fn get_order_seq_state(
        &self,
        store: &Store,
        session_id: SessionId,
    ) -> Arc<Mutex<OrderSeqState>> {
        let mut map = self.order_seq_states.lock().await;
        if let Some(entry) = map.get_mut(&session_id) {
            entry.touch();
            return entry.value.clone();
        }
        let start_seq = store
            .get_session_last_event_seq(session_id)
            .await
            .unwrap_or(0);
        let state = Arc::new(Mutex::new(OrderSeqState::new(start_seq.saturating_add(1))));
        map.insert(session_id, TimedEntry::new(state.clone()));
        state
    }

    pub async fn cached_session_head_snapshot(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
    ) -> Option<SessionHeadSnapshot> {
        let mut cache = self.session_head_cache.lock().await;
        cache
            .get_mut(&session_id)
            .and_then(|entry| {
                entry.touch();
                entry.value.get(&SessionHeadCacheKey {
                    limit,
                    include_events,
                })
            })
            .cloned()
    }

    pub async fn cache_session_head_snapshot(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
        snapshot: SessionHeadSnapshot,
    ) {
        let mut cache = self.session_head_cache.lock().await;
        let entry = cache
            .entry(session_id)
            .or_insert_with(|| TimedEntry::new(HashMap::new()));
        entry.touch();
        entry.value.insert(
            SessionHeadCacheKey {
                limit,
                include_events,
            },
            snapshot,
        );
    }

    pub async fn get_broadcaster(&self, session_id: SessionId) -> broadcast::Sender<SessionEvent> {
        let mut map = self.broadcasters.lock().await;
        let entry = map.entry(session_id).or_insert_with(|| {
            let (tx, _) = broadcast::channel(256);
            TimedEntry::new(tx)
        });
        entry.touch();
        entry.value.clone()
    }

    pub async fn subscribe_session_event_head(
        &self,
        session_id: SessionId,
    ) -> watch::Receiver<i64> {
        let mut map = self.session_event_heads.lock().await;
        if let Some(entry) = map.get_mut(&session_id) {
            entry.touch();
            return entry.value.subscribe();
        }
        let (tx, rx) = watch::channel::<i64>(0);
        map.insert(session_id, TimedEntry::new(tx));
        rx
    }

    pub async fn publish_session_event_head(&self, session_id: SessionId, seq: i64) {
        let mut map = self.session_event_heads.lock().await;
        let sender = map.entry(session_id).or_insert_with(|| {
            let (tx, _rx) = watch::channel::<i64>(0);
            TimedEntry::new(tx)
        });
        sender.touch();
        let _ = sender.value.send(seq);
    }

    pub async fn remember_session_meta(&self, session: &Session) {
        let mut cache = self.session_meta_cache.lock().await;
        cache.insert(session.id, TimedEntry::new(session.clone()));
    }

    async fn cached_session_meta(&self, session_id: SessionId) -> Option<Session> {
        let mut cache = self.session_meta_cache.lock().await;
        cache.get_mut(&session_id).map(|entry| {
            entry.touch();
            entry.value.clone()
        })
    }

    pub async fn session_meta_workspace(&self, session_id: SessionId) -> Option<WorkspaceId> {
        let mut cache = self.session_meta_cache.lock().await;
        cache.get_mut(&session_id).map(|entry| {
            entry.touch();
            entry.value.workspace_id
        })
    }

    pub async fn protected_session_ids(&self) -> HashSet<SessionId> {
        let mut active_sessions: HashSet<SessionId> = HashSet::new();
        {
            let set = self.running_sessions.lock().await;
            active_sessions.extend(set.iter().copied());
        }
        {
            let map = self.schedulers.lock().await;
            active_sessions.extend(map.keys().copied());
        }
        {
            let map = self.broadcasters.lock().await;
            active_sessions.extend(map.keys().copied());
        }
        {
            let map = self.session_event_heads.lock().await;
            active_sessions.extend(map.keys().copied());
        }
        active_sessions
    }

    pub async fn scheduler_sender(
        &self,
        session_id: SessionId,
    ) -> Option<mpsc::Sender<SchedulerCommand>> {
        let mut map = self.schedulers.lock().await;
        map.get_mut(&session_id).map(|entry| {
            entry.touch();
            entry.value.clone()
        })
    }

    pub async fn is_running(&self, session_id: SessionId) -> bool {
        self.running_sessions.lock().await.contains(&session_id)
    }

    pub async fn list_running_sessions(&self) -> Vec<SessionId> {
        self.running_sessions.lock().await.iter().copied().collect()
    }

    pub async fn runtime_stats(&self) -> SessionRuntimeStats {
        SessionRuntimeStats {
            session_head_cache: self.session_head_cache.lock().await.len(),
            session_meta_cache: self.session_meta_cache.lock().await.len(),
            session_event_heads: self.session_event_heads.lock().await.len(),
            schedulers: self.schedulers.lock().await.len(),
            broadcasters: self.broadcasters.lock().await.len(),
            running_sessions: self.running_sessions.lock().await.len(),
            active_task_refreshes: self.active_task_refreshes.lock().await.len(),
        }
    }

    pub async fn sweep_idle_caches(
        &self,
        now: Instant,
        session_ttl: Duration,
    ) -> SessionCacheSweepStats {
        let mut stats = SessionCacheSweepStats::default();
        let running_sessions = {
            let set = self.running_sessions.lock().await;
            set.iter().copied().collect::<HashSet<_>>()
        };
        {
            let mut cache = self.session_head_cache.lock().await;
            let expired: Vec<SessionId> = cache
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                cache.remove(session_id);
            }
            stats.session_head_evicted += expired.len();
        }
        {
            let mut cache = self.session_meta_cache.lock().await;
            let expired: Vec<SessionId> = cache
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                cache.remove(session_id);
            }
            stats.session_meta_evicted += expired.len();
        }
        {
            let mut map = self.schedulers.lock().await;
            let expired: Vec<SessionId> = map
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if entry.value.is_closed()
                        || now.duration_since(entry.last_access) >= session_ttl
                    {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                map.remove(session_id);
            }
            stats.schedulers_evicted += expired.len();
        }
        {
            let mut map = self.broadcasters.lock().await;
            let expired: Vec<SessionId> = map
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                map.remove(session_id);
            }
            stats.broadcasters_evicted += expired.len();
        }
        {
            let mut map = self.session_event_heads.lock().await;
            let expired: Vec<SessionId> = map
                .iter()
                .filter_map(|(session_id, entry)| {
                    if running_sessions.contains(session_id) {
                        return None;
                    }
                    if now.duration_since(entry.last_access) >= session_ttl {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect();
            for session_id in &expired {
                map.remove(session_id);
            }
            stats.session_event_heads_evicted += expired.len();
        }
        stats
    }

    pub async fn remove_session_state(&self, session_id: SessionId) {
        self.session_head_cache.lock().await.remove(&session_id);
        self.schedulers.lock().await.remove(&session_id);
        self.broadcasters.lock().await.remove(&session_id);
        self.session_event_heads.lock().await.remove(&session_id);
        self.running_sessions.lock().await.remove(&session_id);
        self.session_pins.lock().await.remove(&session_id);
        self.session_meta_cache.lock().await.remove(&session_id);
    }

    async fn update_pin_state<F>(&self, session_id: SessionId, update: F) -> Option<bool>
    where
        F: FnOnce(&mut SessionPinState),
    {
        let mut pins = self.session_pins.lock().await;
        let entry = pins.entry(session_id).or_default();
        let was_pinned = entry.is_pinned();
        update(entry);
        let is_pinned = entry.is_pinned();
        if !is_pinned {
            pins.remove(&session_id);
        }
        (was_pinned != is_pinned).then_some(is_pinned)
    }

    pub async fn set_running(&self, session_id: SessionId, running: bool) -> Option<bool> {
        let mut set = self.running_sessions.lock().await;
        let changed = if running {
            set.insert(session_id)
        } else {
            set.remove(&session_id)
        };
        drop(set);
        if !changed {
            return None;
        }
        self.update_pin_state(session_id, |state| state.running = running)
            .await
    }

    pub async fn attach_session(&self, session_id: SessionId) -> Option<bool> {
        self.update_pin_state(session_id, |state| {
            state.attached_clients = state.attached_clients.saturating_add(1);
        })
        .await
    }

    pub async fn detach_session(&self, session_id: SessionId) -> Option<bool> {
        self.update_pin_state(session_id, |state| {
            state.attached_clients = state.attached_clients.saturating_sub(1);
        })
        .await
    }

    pub async fn clear_pin_state(&self, session_id: SessionId) -> bool {
        self.running_sessions.lock().await.remove(&session_id);
        self.session_pins
            .lock()
            .await
            .remove(&session_id)
            .is_some_and(SessionPinState::is_pinned)
    }

    pub async fn provider_inactivity_timeout(&self) -> Duration {
        *self.provider_inactivity_timeout.lock().await
    }

    pub async fn set_provider_inactivity_timeout(&self, timeout: Duration) {
        *self.provider_inactivity_timeout.lock().await = timeout;
    }

    pub async fn set_running_with_host<H>(&self, host: &H, session_id: SessionId, running: bool)
    where
        H: SessionLifecycleHost,
    {
        if let Some(pinned) = self.set_running(session_id, running).await {
            host.set_provider_session_pinned(session_id, pinned).await;
        }
    }

    pub async fn attach_session_with_host<H>(&self, host: &H, session_id: SessionId)
    where
        H: SessionLifecycleHost,
    {
        if let Some(pinned) = self.attach_session(session_id).await {
            host.set_provider_session_pinned(session_id, pinned).await;
        }
    }

    pub async fn detach_session_with_host<H>(&self, host: &H, session_id: SessionId)
    where
        H: SessionLifecycleHost,
    {
        if let Some(pinned) = self.detach_session(session_id).await {
            host.set_provider_session_pinned(session_id, pinned).await;
        }
    }

    pub async fn cleanup_session_with_host<H>(&self, host: &H, session_id: SessionId)
    where
        H: SessionLifecycleHost,
    {
        if self.clear_pin_state(session_id).await {
            host.set_provider_session_pinned(session_id, false).await;
        }
        host.remove_workspace_active_session(session_id).await;
        self.remove_session_state(session_id).await;
    }

    pub async fn publish_event_with_host<H>(&self, host: &H, event: SessionEvent)
    where
        H: SessionEventPublicationHost,
    {
        let tx = self.get_broadcaster(event.session_id).await;
        let _ = tx.send(event.clone());
        self.publish_session_event_head(event.session_id, event.seq)
            .await;

        if is_session_gap_notice(&event) {
            return;
        }

        let session = match self.cached_session_meta(event.session_id).await {
            Some(session) => session,
            None => {
                let Some(session) = host.load_session(event.session_id).await else {
                    return;
                };
                self.remember_session_meta(&session).await;
                session
            }
        };

        if should_refresh_task_delta_for_event(&event.event_type) {
            self.queue_task_delta_refresh_with_host(
                host.task_delta_refresh_host(),
                session.task_id,
            )
            .await;
        }

        let message = if should_materialize_message(&event.event_type) {
            message_from_event(&event, &session)
        } else {
            None
        };

        let tool_event = is_tool_event(&event.event_type);
        let mut tool_summaries = Vec::new();
        if tool_event {
            if let Some(turn_id) = event.turn_id {
                tool_summaries = host
                    .list_turn_tool_summaries_for_turn(event.session_id, turn_id)
                    .await;
            }
        }

        let mut turn = turn_from_event(&event, message.as_ref());
        let prefers_cached_turn = tool_event
            || event_context_window(&event).is_some()
            || should_refresh_turn_from_store(&event.event_type);
        if turn.is_none() && prefers_cached_turn {
            if let Some(turn_id) = event.turn_id {
                turn = host.cached_turn_for_read(event.session_id, turn_id).await;
            }
        }
        if turn.is_none() && (should_refresh_turn_from_store(&event.event_type) || tool_event) {
            if let Some(turn_id) = event.turn_id {
                turn = host.load_turn(event.session_id, turn_id).await;
            }
        }
        if let Some(turn) = turn.as_mut() {
            patch_turn_from_event(turn, &event);
            if !tool_summaries.is_empty() {
                recompute_turn_tool_counts(turn, &tool_summaries);
            }
        }

        let stream_only = is_stream_only_event(&event.event_type);
        let cached_replay_cursor = if stream_only {
            host.session_replay_cursor(session.workspace_id, event.session_id)
                .await
        } else {
            SessionReplayCursor::default()
        };
        let last_event_seq = if stream_only {
            cached_replay_cursor.last_event_seq
        } else {
            event.seq
        };
        let projection_rev = resolve_projection_rev_for_stream_delta(
            stream_only,
            last_event_seq,
            cached_replay_cursor.projection_rev,
            || async { host.load_projection_rev(event.session_id).await },
        )
        .await;
        let state_rev = last_event_seq;

        let activity =
            derive_summary_activity(&event).or_else(|| turn.as_ref().map(activity_from_turn));

        let mut last_message_at = None;
        let mut last_message_preview = None;
        if let Some(message) = message.as_ref() {
            last_message_at = Some(message.created_at);
            last_message_preview = Some(derive_message_preview(&message.content));
        }

        let summary_delta = build_session_summary_delta(
            &session,
            activity.clone(),
            last_message_at,
            last_message_preview,
            last_event_seq,
            projection_rev,
            state_rev,
        );

        let delta = SessionHeadDelta {
            session_id: event.session_id,
            last_event_seq,
            projection_rev,
            state_rev,
            emitted_at_ms: Some(chrono::Utc::now().timestamp_millis()),
            session: should_include_session_metadata_in_head_delta(&event.event_type)
                .then(|| session_metadata_from_session(&session)),
            activity,
            event: Some(event),
            turn,
            message,
            tool_summaries,
        };
        host.publish_session_head_delta(session.workspace_id, &session, delta, !stream_only)
            .await;

        if let Some(summary_delta) = summary_delta {
            host.publish_session_summary_delta(session.workspace_id, summary_delta)
                .await;
        }
    }

    pub async fn queue_task_delta_refresh_with_host<H>(&self, host: Arc<H>, task_id: TaskId)
    where
        H: SessionTaskDeltaRefreshHost,
    {
        self.queue_task_delta_refresh_with_debounce(
            host,
            task_id,
            Duration::from_millis(TASK_DELTA_REFRESH_DEBOUNCE_MS.max(1)),
        )
        .await;
    }

    async fn queue_task_delta_refresh_with_debounce<H>(
        &self,
        host: Arc<H>,
        task_id: TaskId,
        debounce: Duration,
    ) where
        H: SessionTaskDeltaRefreshHost,
    {
        let should_spawn = {
            let mut map = self.active_task_refreshes.lock().await;
            if let Some(entry) = map.get_mut(&task_id) {
                entry.generation = entry.generation.wrapping_add(1);
                false
            } else {
                map.insert(task_id, ActiveTaskRefreshEntry { generation: 1 });
                true
            }
        };
        if should_spawn {
            let active_task_refreshes = Arc::clone(&self.active_task_refreshes);
            tokio::spawn(async move {
                run_task_delta_refresh_loop(active_task_refreshes, host, task_id, debounce).await;
            });
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SessionReplayCursor {
    pub last_event_seq: i64,
    pub projection_rev: i64,
}

#[async_trait]
pub trait SessionEventPublicationHost: Send + Sync {
    type TaskDeltaRefreshHost: SessionTaskDeltaRefreshHost;

    fn task_delta_refresh_host(&self) -> Arc<Self::TaskDeltaRefreshHost>;

    async fn load_session(&self, session_id: SessionId) -> Option<Session>;

    async fn list_turn_tool_summaries_for_turn(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Vec<SessionTurnToolSummary>;

    async fn cached_turn_for_read(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Option<SessionTurn>;

    async fn load_turn(&self, session_id: SessionId, turn_id: TurnId) -> Option<SessionTurn>;

    async fn session_replay_cursor(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> SessionReplayCursor;

    async fn load_projection_rev(&self, session_id: SessionId) -> Option<i64>;

    async fn publish_session_head_delta(
        &self,
        workspace_id: WorkspaceId,
        session: &Session,
        delta: SessionHeadDelta,
        durable: bool,
    );

    async fn publish_session_summary_delta(
        &self,
        workspace_id: WorkspaceId,
        delta: SessionSummaryDelta,
    );
}

#[async_trait]
pub trait SessionTaskDeltaRefreshHost: Send + Sync + 'static {
    async fn emit_task_delta_refresh(&self, task_id: TaskId);
}

#[async_trait]
pub trait SessionLifecycleHost: Send + Sync {
    async fn set_provider_session_pinned(&self, session_id: SessionId, pinned: bool);

    async fn remove_workspace_active_session(&self, session_id: SessionId);
}

async fn run_task_delta_refresh_loop<H>(
    active_task_refreshes: Arc<Mutex<HashMap<TaskId, ActiveTaskRefreshEntry>>>,
    host: Arc<H>,
    task_id: TaskId,
    debounce: Duration,
) where
    H: SessionTaskDeltaRefreshHost,
{
    let debounce = debounce.max(Duration::from_millis(1));
    loop {
        let generation = {
            let map = active_task_refreshes.lock().await;
            match map.get(&task_id) {
                Some(entry) => entry.generation,
                None => return,
            }
        };

        tokio::time::sleep(debounce).await;

        let current = {
            let map = active_task_refreshes.lock().await;
            match map.get(&task_id) {
                Some(entry) => entry.generation,
                None => return,
            }
        };
        if current != generation {
            continue;
        }

        host.emit_task_delta_refresh(task_id).await;

        let mut map = active_task_refreshes.lock().await;
        match map.get(&task_id) {
            Some(entry) if entry.generation == current => {
                map.remove(&task_id);
                return;
            }
            Some(_) => continue,
            None => return,
        }
    }
}

fn is_stream_only_event(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::AssistantChunk
            | SessionEventType::ThoughtChunk
            | SessionEventType::ContextWindowUpdate
    )
}

fn should_refresh_task_delta_for_event(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::UserMessage
            | SessionEventType::AssistantMessageInserted
            | SessionEventType::AssistantComplete
            | SessionEventType::Done
            | SessionEventType::TurnQueued
            | SessionEventType::TurnStarted
            | SessionEventType::TurnFinished
            | SessionEventType::TurnInterrupted
            | SessionEventType::MessageQueueAdded
            | SessionEventType::MessageQueueUpdated
            | SessionEventType::MessageQueueRemoved
            | SessionEventType::MessageQueuePromoted
            | SessionEventType::Error
    )
}

fn should_materialize_message(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::UserMessage
            | SessionEventType::AssistantMessageInserted
            | SessionEventType::Notice
    )
}

fn is_tool_event(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::ToolCall
            | SessionEventType::ToolCallUpdate
            | SessionEventType::ToolResult
    )
}

impl<SchedulerCommand: Send + 'static> SessionRuntime<SchedulerCommand> {
    pub async fn ensure_scheduler<F, Fut>(
        &self,
        session: Session,
        spawn_worker: F,
    ) -> mpsc::Sender<SchedulerCommand>
    where
        F: FnOnce(Session, mpsc::Receiver<SchedulerCommand>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.remember_session_meta(&session).await;
        let mut map = self.schedulers.lock().await;
        if let Some(entry) = map.get_mut(&session.id) {
            if !entry.value.is_closed() {
                entry.touch();
                return entry.value.clone();
            }
            map.remove(&session.id);
            tracing::info!(
                session_id = %session.id.0,
                "scheduler sender closed; recreating"
            );
        }
        let (tx, rx) = mpsc::channel(64);
        map.insert(session.id, TimedEntry::new(tx.clone()));
        drop(map);
        tokio::spawn(spawn_worker(session, rx));
        tx
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SessionPinState {
    pub running: bool,
    pub attached_clients: usize,
}

impl SessionPinState {
    pub fn is_pinned(self) -> bool {
        self.running || self.attached_clients > 0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionHeadCacheKey {
    pub limit: u32,
    pub include_events: bool,
}

#[derive(Debug)]
pub struct TimedEntry<T> {
    pub value: T,
    pub last_access: Instant,
}

impl<T> TimedEntry<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            last_access: Instant::now(),
        }
    }

    pub fn touch(&mut self) {
        self.last_access = Instant::now();
    }

    pub fn touch_at(&mut self, now: Instant) {
        self.last_access = now;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ActiveTaskRefreshEntry {
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SessionCacheSweepStats {
    pub session_head_evicted: usize,
    pub session_meta_evicted: usize,
    pub schedulers_evicted: usize,
    pub broadcasters_evicted: usize,
    pub session_event_heads_evicted: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SessionRuntimeStats {
    pub session_head_cache: usize,
    pub session_meta_cache: usize,
    pub session_event_heads: usize,
    pub schedulers: usize,
    pub broadcasters: usize,
    pub running_sessions: usize,
    pub active_task_refreshes: usize,
}

pub fn provider_inactivity_timeout_from_env() -> Duration {
    std::env::var("CTX_PROVIDER_TURN_INACTIVITY_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_PROVIDER_INACTIVITY_TIMEOUT_SECS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ctx_core::ids::{MessageId, SessionEventId, TurnId, WorktreeId};
    use ctx_core::models::{
        ExecutionEnvironment, SessionHeadDelta, SessionStatus, SessionSummaryDelta,
        SessionTurnStatus,
    };
    use serde_json::json;

    #[tokio::test]
    async fn pin_state_reports_only_pinned_transitions() {
        let runtime = SessionRuntime::<()>::new(Duration::from_secs(60));
        let session_id = SessionId::new();

        assert_eq!(runtime.set_running(session_id, true).await, Some(true));
        assert_eq!(runtime.set_running(session_id, true).await, None);
        assert_eq!(runtime.attach_session(session_id).await, None);
        assert_eq!(runtime.set_running(session_id, false).await, None);
        assert_eq!(runtime.detach_session(session_id).await, Some(false));
    }

    #[tokio::test]
    async fn lifecycle_host_receives_only_pin_transitions_and_cleanup() {
        let runtime = SessionRuntime::<()>::new(Duration::from_secs(60));
        let host = RecordingLifecycleHost::default();
        let session_id = SessionId::new();

        runtime.set_running_with_host(&host, session_id, true).await;
        runtime.set_running_with_host(&host, session_id, true).await;
        runtime.attach_session_with_host(&host, session_id).await;
        runtime
            .set_running_with_host(&host, session_id, false)
            .await;
        runtime.detach_session_with_host(&host, session_id).await;
        runtime.cleanup_session_with_host(&host, session_id).await;

        assert_eq!(
            host.pin_updates.lock().await.as_slice(),
            &[(session_id, true), (session_id, false)]
        );
        assert_eq!(host.removed_sessions.lock().await.as_slice(), &[session_id]);
        assert!(!runtime.is_running(session_id).await);
        assert!(runtime.session_meta_workspace(session_id).await.is_none());
    }

    #[tokio::test]
    async fn task_session_creation_lock_reuses_live_lock_and_prunes_dead_entries() {
        let runtime = SessionRuntime::<()>::new(Duration::from_secs(60));
        let task_id = TaskId::new();

        let first = runtime.task_session_creation_lock(task_id).await;
        let second = runtime.task_session_creation_lock(task_id).await;
        assert!(Arc::ptr_eq(&first, &second));

        drop(first);
        drop(second);

        let replacement = runtime.task_session_creation_lock(task_id).await;
        assert_eq!(Arc::strong_count(&replacement), 1);
    }

    #[tokio::test]
    async fn publish_gap_notice_only_updates_realtime_session_channels() {
        let runtime = SessionRuntime::<()>::new(Duration::from_secs(60));
        let host = RecordingPublicationHost::default();
        let session_id = SessionId::new();
        let mut events = runtime.get_broadcaster(session_id).await.subscribe();
        let head = runtime.subscribe_session_event_head(session_id).await;
        let event = test_event(
            session_id,
            SessionEventType::Notice,
            json!({
                "kind": "session_gap",
                "reason": "data_plane_overflow"
            }),
        );

        runtime.publish_event_with_host(&host, event.clone()).await;

        assert_eq!(events.try_recv().expect("published event").id, event.id);
        assert_eq!(*head.borrow(), event.seq);
        assert_eq!(host.load_session_calls.lock().await.len(), 0);
        assert!(host.head_deltas.lock().await.is_empty());
        assert!(host.summary_deltas.lock().await.is_empty());
    }

    #[tokio::test]
    async fn publish_user_message_materializes_head_summary_and_task_delta() {
        let runtime = SessionRuntime::<()>::new(Duration::from_secs(60));
        let session = test_session();
        let host = RecordingPublicationHost::new(session.clone());
        *host.projection_rev.lock().await = Some(42);
        let mut event = test_event(session.id, SessionEventType::UserMessage, json!({}));
        let turn_id = TurnId::new();
        let message_id = MessageId::new();
        event.turn_id = Some(turn_id);
        event.seq = 12;
        event.payload_json = json!({
            "message_id": message_id.0.to_string(),
            "content": "first line\nsecond line",
            "order_seq": 3
        });

        runtime.publish_event_with_host(&host, event).await;

        let head_deltas = host.head_deltas.lock().await;
        assert_eq!(head_deltas.len(), 1);
        let published = &head_deltas[0];
        assert_eq!(published.workspace_id, session.workspace_id);
        assert!(published.durable);
        assert_eq!(published.delta.last_event_seq, 12);
        assert_eq!(published.delta.projection_rev, 42);
        assert_eq!(
            published.delta.message.as_ref().map(|message| message.id),
            Some(message_id)
        );
        assert_eq!(
            published
                .delta
                .turn
                .as_ref()
                .map(|turn| turn.status.clone()),
            Some(SessionTurnStatus::Starting)
        );
        drop(head_deltas);

        let summary_deltas = host.summary_deltas.lock().await;
        assert_eq!(summary_deltas.len(), 1);
        assert_eq!(
            summary_deltas[0].last_message_preview.as_deref(),
            Some("first line")
        );
        drop(summary_deltas);

        wait_for_task_delta(&host.task_delta_refresh_host).await;
        assert_eq!(
            host.task_delta_refresh_host
                .task_ids
                .lock()
                .await
                .as_slice(),
            &[session.task_id]
        );
    }

    #[tokio::test]
    async fn stream_only_event_uses_replay_cursor_and_stays_transient() {
        let runtime = SessionRuntime::<()>::new(Duration::from_secs(60));
        let session = test_session();
        let host = RecordingPublicationHost::new(session.clone());
        *host.replay_cursor.lock().await = SessionReplayCursor {
            last_event_seq: 25,
            projection_rev: 9,
        };
        let mut event = test_event(
            session.id,
            SessionEventType::AssistantChunk,
            json!({"delta": "partial"}),
        );
        event.seq = 30;

        runtime.publish_event_with_host(&host, event).await;

        let head_deltas = host.head_deltas.lock().await;
        assert_eq!(head_deltas.len(), 1);
        assert!(!head_deltas[0].durable);
        assert_eq!(head_deltas[0].delta.last_event_seq, 25);
        assert_eq!(head_deltas[0].delta.projection_rev, 9);
        drop(head_deltas);
        assert_eq!(*host.projection_rev_loads.lock().await, 0);
    }

    #[tokio::test]
    async fn task_delta_refresh_debounces_to_latest_generation() {
        let runtime = SessionRuntime::<()>::new(Duration::from_secs(60));
        let task_id = TaskId::new();
        let host = Arc::new(RecordingTaskDeltaRefreshHost::default());

        runtime
            .queue_task_delta_refresh_with_debounce(
                Arc::clone(&host),
                task_id,
                Duration::from_millis(1),
            )
            .await;
        runtime
            .queue_task_delta_refresh_with_debounce(
                Arc::clone(&host),
                task_id,
                Duration::from_millis(1),
            )
            .await;

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !host.task_ids.lock().await.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("task delta refresh should fire");

        assert_eq!(host.task_ids.lock().await.as_slice(), &[task_id]);
        assert!(runtime.active_task_refreshes.lock().await.is_empty());
    }

    async fn wait_for_task_delta(host: &RecordingTaskDeltaRefreshHost) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !host.task_ids.lock().await.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("task delta refresh should fire");
    }

    fn test_session() -> Session {
        let now = Utc::now();
        Session {
            id: SessionId::new(),
            task_id: TaskId::new(),
            workspace_id: WorkspaceId::new(),
            worktree_id: WorktreeId::new(),
            execution_environment: ExecutionEnvironment::Host,
            parent_session_id: None,
            relationship: None,
            provider_id: "fake".to_string(),
            model_id: "fake-model".to_string(),
            reasoning_effort: None,
            title: "Test".to_string(),
            agent_role: "assistant".to_string(),
            status: SessionStatus::Active,
            provider_session_ref: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_event(
        session_id: SessionId,
        event_type: SessionEventType,
        payload_json: serde_json::Value,
    ) -> SessionEvent {
        SessionEvent {
            seq: 7,
            id: SessionEventId::new(),
            session_id,
            run_id: None,
            turn_id: None,
            event_type,
            payload_json,
            transient: false,
            created_at: Utc::now(),
        }
    }

    #[derive(Default)]
    struct RecordingPublicationHost {
        session: Mutex<Option<Session>>,
        task_delta_refresh_host: Arc<RecordingTaskDeltaRefreshHost>,
        replay_cursor: Mutex<SessionReplayCursor>,
        projection_rev: Mutex<Option<i64>>,
        projection_rev_loads: Mutex<usize>,
        load_session_calls: Mutex<Vec<SessionId>>,
        head_deltas: Mutex<Vec<PublishedHeadDelta>>,
        summary_deltas: Mutex<Vec<SessionSummaryDelta>>,
    }

    impl RecordingPublicationHost {
        fn new(session: Session) -> Self {
            Self {
                session: Mutex::new(Some(session)),
                ..Self::default()
            }
        }
    }

    struct PublishedHeadDelta {
        workspace_id: WorkspaceId,
        delta: SessionHeadDelta,
        durable: bool,
    }

    #[async_trait]
    impl SessionEventPublicationHost for RecordingPublicationHost {
        type TaskDeltaRefreshHost = RecordingTaskDeltaRefreshHost;

        fn task_delta_refresh_host(&self) -> Arc<Self::TaskDeltaRefreshHost> {
            Arc::clone(&self.task_delta_refresh_host)
        }

        async fn load_session(&self, session_id: SessionId) -> Option<Session> {
            self.load_session_calls.lock().await.push(session_id);
            self.session.lock().await.clone()
        }

        async fn list_turn_tool_summaries_for_turn(
            &self,
            _session_id: SessionId,
            _turn_id: TurnId,
        ) -> Vec<SessionTurnToolSummary> {
            Vec::new()
        }

        async fn cached_turn_for_read(
            &self,
            _session_id: SessionId,
            _turn_id: TurnId,
        ) -> Option<SessionTurn> {
            None
        }

        async fn load_turn(&self, _session_id: SessionId, _turn_id: TurnId) -> Option<SessionTurn> {
            None
        }

        async fn session_replay_cursor(
            &self,
            _workspace_id: WorkspaceId,
            _session_id: SessionId,
        ) -> SessionReplayCursor {
            *self.replay_cursor.lock().await
        }

        async fn load_projection_rev(&self, _session_id: SessionId) -> Option<i64> {
            *self.projection_rev_loads.lock().await += 1;
            *self.projection_rev.lock().await
        }

        async fn publish_session_head_delta(
            &self,
            workspace_id: WorkspaceId,
            _session: &Session,
            delta: SessionHeadDelta,
            durable: bool,
        ) {
            self.head_deltas.lock().await.push(PublishedHeadDelta {
                workspace_id,
                delta,
                durable,
            });
        }

        async fn publish_session_summary_delta(
            &self,
            _workspace_id: WorkspaceId,
            delta: SessionSummaryDelta,
        ) {
            self.summary_deltas.lock().await.push(delta);
        }
    }

    #[derive(Default)]
    struct RecordingTaskDeltaRefreshHost {
        task_ids: Mutex<Vec<TaskId>>,
    }

    #[async_trait]
    impl SessionTaskDeltaRefreshHost for RecordingTaskDeltaRefreshHost {
        async fn emit_task_delta_refresh(&self, task_id: TaskId) {
            self.task_ids.lock().await.push(task_id);
        }
    }

    #[derive(Default)]
    struct RecordingLifecycleHost {
        pin_updates: Mutex<Vec<(SessionId, bool)>>,
        removed_sessions: Mutex<Vec<SessionId>>,
    }

    #[async_trait]
    impl SessionLifecycleHost for RecordingLifecycleHost {
        async fn set_provider_session_pinned(&self, session_id: SessionId, pinned: bool) {
            self.pin_updates.lock().await.push((session_id, pinned));
        }

        async fn remove_workspace_active_session(&self, session_id: SessionId) {
            self.removed_sessions.lock().await.push(session_id);
        }
    }
}
