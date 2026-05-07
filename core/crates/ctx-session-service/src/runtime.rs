use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, mpsc, watch, Mutex};

use ctx_core::ids::{SessionId, TaskId, WorkspaceId};
use ctx_core::models::{Session, SessionEvent, SessionHeadSnapshot};
use ctx_session_tools::order_seq::OrderSeqState;
use ctx_store::Store;

const DEFAULT_PROVIDER_INACTIVITY_TIMEOUT_SECS: u64 = 30 * 60;

pub struct SessionRuntime<SchedulerCommand> {
    pub session_head_cache:
        Mutex<HashMap<SessionId, TimedEntry<HashMap<SessionHeadCacheKey, SessionHeadSnapshot>>>>,
    pub schedulers: Mutex<HashMap<SessionId, TimedEntry<mpsc::Sender<SchedulerCommand>>>>,
    pub provider_inactivity_timeout: Mutex<Duration>,
    pub broadcasters: Mutex<HashMap<SessionId, TimedEntry<broadcast::Sender<SessionEvent>>>>,
    pub session_event_heads: Mutex<HashMap<SessionId, TimedEntry<watch::Sender<i64>>>>,
    pub order_seq_states: Mutex<HashMap<SessionId, TimedEntry<Arc<Mutex<OrderSeqState>>>>>,
    pub active_task_refreshes: Mutex<HashMap<TaskId, ActiveTaskRefreshEntry>>,
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
            active_task_refreshes: Mutex::new(HashMap::new()),
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
}
