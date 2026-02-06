use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, mpsc, watch, Mutex};

use ctx_core::ids::{SessionId, TaskId};
use ctx_core::models::{
    Message, MessageAttachment, MessageDelivery, MessageRole, Session, SessionEvent,
    SessionEventType, SessionHeadDelta, SessionHeadSnapshot, SessionTurn, SessionTurnStatus,
    SessionTurnToolSummary,
};
use ctx_store::Store;

use crate::order_seq::OrderSeqState;
use crate::scheduler::session_worker;

use super::state::{
    ActiveHeadProjectionEntry, ActiveTaskRefreshEntry, AppState, SessionHeadCacheKey,
    SessionRuntime, TimedEntry,
};

const ACTIVE_HEAD_PROJECTION_DEBOUNCE_MS: u64 = 200;
const ACTIVE_HEAD_PROJECTION_MAX_FLUSH_MS: u64 = 1500;
const ACTIVE_TASK_REFRESH_DEBOUNCE_MS: u64 = 250;

fn active_head_projection_wait_duration(
    now: Instant,
    last_event_at: Instant,
    last_flush_at: Instant,
    debounce: Duration,
    max_flush: Duration,
) -> Duration {
    let wait_for_debounce = debounce.saturating_sub(now.duration_since(last_event_at));
    let wait_for_max = max_flush.saturating_sub(now.duration_since(last_flush_at));
    wait_for_debounce.min(wait_for_max)
}

fn active_head_projection_should_flush(
    now: Instant,
    last_event_at: Instant,
    last_flush_at: Instant,
    debounce: Duration,
    max_flush: Duration,
) -> bool {
    now.duration_since(last_event_at) >= debounce || now.duration_since(last_flush_at) >= max_flush
}

fn message_from_event(event: &SessionEvent, session: &Session) -> Option<Message> {
    let message_id = event
        .payload_json
        .get("message_id")
        .and_then(|v| v.as_str())
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .map(ctx_core::ids::MessageId)?;
    let content = event
        .payload_json
        .get("content")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())?;
    let delivery = event
        .payload_json
        .get("delivery")
        .and_then(|v| serde_json::from_value::<MessageDelivery>(v.clone()).ok())
        .unwrap_or(MessageDelivery::Immediate);
    let attachments = event
        .payload_json
        .get("attachments")
        .and_then(|v| serde_json::from_value::<Vec<MessageAttachment>>(v.clone()).ok())
        .unwrap_or_default();
    let order_seq = event
        .payload_json
        .get("order_seq")
        .or_else(|| event.payload_json.get("orderSeq"))
        .and_then(|v| v.as_i64());
    let role = match event.event_type {
        SessionEventType::UserMessage => MessageRole::User,
        SessionEventType::AssistantMessageInserted => MessageRole::Assistant,
        _ => return None,
    };
    let delivered_at = match role {
        MessageRole::Assistant => Some(event.created_at),
        _ => None,
    };
    Some(Message {
        id: message_id,
        session_id: event.session_id,
        task_id: session.task_id,
        run_id: event.run_id,
        turn_id: event.turn_id,
        turn_sequence: event
            .payload_json
            .get("turn_sequence")
            .and_then(|v| v.as_i64()),
        order_seq,
        role,
        content,
        attachments,
        delivery,
        delivered_at,
        created_at: event.created_at,
    })
}

fn turn_from_event(event: &SessionEvent, message: Option<&Message>) -> Option<SessionTurn> {
    if !matches!(event.event_type, SessionEventType::UserMessage) {
        return None;
    }
    let turn_id = event.turn_id?;
    let delivery = message
        .map(|msg| msg.delivery.clone())
        .unwrap_or(MessageDelivery::Immediate);
    let status = if matches!(delivery, MessageDelivery::Queued) {
        SessionTurnStatus::Queued
    } else {
        SessionTurnStatus::Running
    };
    Some(SessionTurn {
        turn_id,
        session_id: event.session_id,
        run_id: event.run_id,
        user_message_id: message.map(|msg| msg.id),
        status,
        start_seq: Some(event.seq),
        end_seq: None,
        started_at: event.created_at,
        updated_at: event.created_at,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    })
}

impl SessionRuntime {
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

    pub async fn publish_event(&self, state: &Arc<AppState>, event: SessionEvent) {
        let tx = self.get_broadcaster(event.session_id).await;
        let _ = tx.send(event.clone());
        let mut map = self.session_event_heads.lock().await;
        let sender = map.entry(event.session_id).or_insert_with(|| {
            let (tx, _rx) = watch::channel::<i64>(0);
            TimedEntry::new(tx)
        });
        sender.touch();
        let _ = sender.value.send(event.seq);
        if !matches!(
            event.event_type,
            SessionEventType::AssistantChunk | SessionEventType::ThoughtChunk
        ) {
            self.queue_active_head_projection(state, event.session_id, event.seq)
                .await;
        }
        self.update_workspace_active_snapshot_for_event(state, &event)
            .await;
    }

    async fn update_workspace_active_snapshot_for_event(
        &self,
        state: &Arc<AppState>,
        event: &SessionEvent,
    ) {
        let session = {
            let mut cache = self.session_meta_cache.lock().await;
            cache.get_mut(&event.session_id).map(|entry| {
                entry.touch();
                entry.value.clone()
            })
        };
        let session = match session {
            Some(session) => session,
            None => {
                let store = match state.store_for_session(event.session_id).await {
                    Ok(store) => store,
                    Err(_) => return,
                };
                let Some(session) = store.get_session(event.session_id).await.ok().flatten() else {
                    return;
                };
                self.remember_session_meta(&session).await;
                session
            }
        };

        let stream_only = matches!(
            event.event_type,
            SessionEventType::AssistantChunk | SessionEventType::ThoughtChunk
        );

        let update_task = matches!(
            event.event_type,
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
        );
        if update_task {
            self.queue_workspace_task_refresh(state, session.task_id)
                .await;
        }

        let message = if matches!(
            event.event_type,
            SessionEventType::UserMessage
                | SessionEventType::AssistantMessageInserted
                | SessionEventType::Notice
        ) {
            message_from_event(event, &session)
        } else {
            None
        };
        let mut turn = turn_from_event(event, message.as_ref());
        if turn.is_none()
            && matches!(
                event.event_type,
                SessionEventType::TurnStarted | SessionEventType::TurnFinished
            )
        {
            if let Some(turn_id) = event.turn_id {
                if let Ok(store) = state.store_for_session(event.session_id).await {
                    if let Ok(Some(fetched)) =
                        store.get_session_turn(event.session_id, turn_id).await
                    {
                        turn = Some(fetched);
                    }
                }
            }
        }
        let mut tool_summaries: Vec<SessionTurnToolSummary> = Vec::new();
        if matches!(
            event.event_type,
            SessionEventType::ToolCall
                | SessionEventType::ToolCallUpdate
                | SessionEventType::ToolResult
        ) {
            if let Some(turn_id) = event.turn_id {
                if let Ok(store) = state.store_for_session(event.session_id).await {
                    if let Ok(list) = store
                        .list_turn_tool_summaries_for_turns(
                            event.session_id,
                            std::slice::from_ref(&turn_id),
                        )
                        .await
                    {
                        tool_summaries = list;
                    }
                }
            }
        }

        let last_event_seq = if stream_only {
            state
                .workspaces
                .workspace_active_snapshot
                .session_last_event_seq(session.workspace_id, event.session_id)
                .await
        } else {
            event.seq
        };
        let state_rev = if stream_only {
            last_event_seq
        } else {
            event.seq
        };

        let delta = SessionHeadDelta {
            session_id: event.session_id,
            last_event_seq,
            state_rev,
            event: Some(event.clone()),
            turn,
            message,
            tool_summaries,
        };
        state
            .workspaces
            .workspace_active_snapshot
            .publish_session_head_delta(session.workspace_id, &session, delta, !stream_only)
            .await;
    }

    async fn queue_active_head_projection(
        &self,
        state: &Arc<AppState>,
        session_id: SessionId,
        last_event_seq: i64,
    ) {
        let now = Instant::now();
        let should_spawn = {
            let mut map = self.active_head_projections.lock().await;
            if let Some(entry) = map.get_mut(&session_id) {
                entry.last_event_seq = entry.last_event_seq.max(last_event_seq);
                entry.last_event_at = now;
                false
            } else {
                map.insert(
                    session_id,
                    ActiveHeadProjectionEntry {
                        last_event_seq,
                        last_event_at: now,
                        last_flushed_seq: 0,
                        last_flush_at: now,
                    },
                );
                true
            }
        };
        if should_spawn {
            let state = Arc::downgrade(state);
            tokio::spawn(async move {
                let Some(state) = state.upgrade() else {
                    return;
                };
                let state_clone = state.clone();
                state
                    .sessions
                    .run_active_head_projection(state_clone, session_id)
                    .await;
            });
        }
    }

    async fn queue_workspace_task_refresh(&self, state: &Arc<AppState>, task_id: TaskId) {
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
            let state = Arc::downgrade(state);
            tokio::spawn(async move {
                let Some(state) = state.upgrade() else {
                    return;
                };
                let state_clone = state.clone();
                state
                    .sessions
                    .run_workspace_task_refresh(state_clone, task_id)
                    .await;
            });
        }
    }

    async fn run_active_head_projection(&self, state: Arc<AppState>, session_id: SessionId) {
        let debounce = Duration::from_millis(ACTIVE_HEAD_PROJECTION_DEBOUNCE_MS.max(1));
        let max_flush = Duration::from_millis(ACTIVE_HEAD_PROJECTION_MAX_FLUSH_MS.max(1));
        loop {
            let entry = {
                let map = self.active_head_projections.lock().await;
                map.get(&session_id).cloned()
            };
            let Some(entry) = entry else {
                return;
            };
            if entry.last_event_seq == entry.last_flushed_seq {
                tokio::time::sleep(debounce).await;
                let mut map = self.active_head_projections.lock().await;
                if let Some(entry) = map.get(&session_id) {
                    if entry.last_event_seq == entry.last_flushed_seq {
                        map.remove(&session_id);
                        return;
                    }
                } else {
                    return;
                }
                continue;
            }

            let wait_for = active_head_projection_wait_duration(
                Instant::now(),
                entry.last_event_at,
                entry.last_flush_at,
                debounce,
                max_flush,
            );
            if !wait_for.is_zero() {
                tokio::time::sleep(wait_for).await;
            }

            let entry = {
                let map = self.active_head_projections.lock().await;
                map.get(&session_id).cloned()
            };
            let Some(entry) = entry else {
                return;
            };
            if entry.last_event_seq == entry.last_flushed_seq {
                continue;
            }
            let now = Instant::now();
            if !active_head_projection_should_flush(
                now,
                entry.last_event_at,
                entry.last_flush_at,
                debounce,
                max_flush,
            ) {
                continue;
            }
            let target_seq = entry.last_event_seq;

            self.refresh_session_head_cache(&state, session_id).await;

            let flushed_at = Instant::now();
            let mut map = self.active_head_projections.lock().await;
            match map.get_mut(&session_id) {
                Some(entry) => {
                    entry.last_flushed_seq = entry.last_flushed_seq.max(target_seq);
                    entry.last_flush_at = flushed_at;
                    if entry.last_event_seq == entry.last_flushed_seq {
                        map.remove(&session_id);
                        return;
                    }
                }
                None => return,
            }
        }
    }

    async fn run_workspace_task_refresh(&self, state: Arc<AppState>, task_id: TaskId) {
        let debounce = Duration::from_millis(ACTIVE_TASK_REFRESH_DEBOUNCE_MS.max(1));
        loop {
            let generation = {
                let map = self.active_task_refreshes.lock().await;
                match map.get(&task_id) {
                    Some(entry) => entry.generation,
                    None => return,
                }
            };

            tokio::time::sleep(debounce).await;

            let current = {
                let map = self.active_task_refreshes.lock().await;
                match map.get(&task_id) {
                    Some(entry) => entry.generation,
                    None => return,
                }
            };
            if current != generation {
                continue;
            }

            if let Err(err) = state.emit_workspace_task_upsert(task_id).await {
                tracing::warn!(
                    task_id = %task_id.0,
                    "workspace active snapshot refresh failed: {err:?}"
                );
            }

            let mut map = self.active_task_refreshes.lock().await;
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

    pub async fn remember_session_meta(&self, session: &Session) {
        let mut cache = self.session_meta_cache.lock().await;
        cache.insert(session.id, TimedEntry::new(session.clone()));
    }

    pub async fn refresh_session_head_cache(&self, state: &AppState, session_id: SessionId) {
        let store = match state.store_for_session(session_id).await {
            Ok(store) => store,
            Err(_) => return,
        };
        // Avoid unbounded head refresh: extremely large conversations can turn the
        // workspace active heads snapshot into multi-megabyte payloads, which then
        // backpressure the workspace WS stream.
        const SESSION_HEAD_REFRESH_TURN_LIMIT: u32 = 200;
        let head = match store
            .get_session_head_snapshot(session_id, SESSION_HEAD_REFRESH_TURN_LIMIT, true)
            .await
        {
            Ok(Some(head)) => head,
            Ok(None) => {
                state
                    .workspaces
                    .workspace_active_snapshot
                    .remove_session_head(session_id)
                    .await;
                return;
            }
            Err(err) => {
                tracing::warn!(
                    session_id = %session_id.0,
                    "session head cache refresh failed: {err:#}"
                );
                return;
            }
        };
        state
            .workspaces
            .workspace_active_snapshot
            .update_session_head(head)
            .await;
    }

    pub async fn ensure_scheduler(
        &self,
        state: &Arc<AppState>,
        session: Session,
    ) -> mpsc::Sender<crate::scheduler::SchedulerCommand> {
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
        tokio::spawn(session_worker(state.clone(), session, rx));
        tx
    }

    pub async fn scheduler_sender(
        &self,
        session_id: SessionId,
    ) -> Option<mpsc::Sender<crate::scheduler::SchedulerCommand>> {
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

    pub async fn set_running(&self, session_id: SessionId, running: bool) {
        let mut set = self.running_sessions.lock().await;
        if running {
            set.insert(session_id);
        } else {
            set.remove(&session_id);
        }
    }

    pub async fn cleanup_session(&self, state: &AppState, session_id: SessionId) {
        state
            .workspaces
            .workspace_active_snapshot
            .remove_session(session_id)
            .await;
        {
            let mut cache = self.session_head_cache.lock().await;
            cache.remove(&session_id);
        }
        {
            let mut map = self.schedulers.lock().await;
            map.remove(&session_id);
        }
        {
            let mut map = self.broadcasters.lock().await;
            map.remove(&session_id);
        }
        {
            let mut map = self.session_event_heads.lock().await;
            map.remove(&session_id);
        }
        {
            let mut map = self.active_head_projections.lock().await;
            map.remove(&session_id);
        }
        {
            let mut set = self.running_sessions.lock().await;
            set.remove(&session_id);
        }
        {
            let mut cache = self.session_meta_cache.lock().await;
            cache.remove(&session_id);
        }
    }
}

impl AppState {
    pub async fn cached_session_head_snapshot(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
    ) -> Option<SessionHeadSnapshot> {
        self.sessions
            .cached_session_head_snapshot(session_id, limit, include_events)
            .await
    }

    pub async fn cache_session_head_snapshot(
        &self,
        session_id: SessionId,
        limit: u32,
        include_events: bool,
        snapshot: SessionHeadSnapshot,
    ) {
        self.sessions
            .cache_session_head_snapshot(session_id, limit, include_events, snapshot)
            .await;
    }

    pub async fn get_broadcaster(&self, session_id: SessionId) -> broadcast::Sender<SessionEvent> {
        self.sessions.get_broadcaster(session_id).await
    }

    pub async fn subscribe_session_event_head(
        &self,
        session_id: SessionId,
    ) -> watch::Receiver<i64> {
        self.sessions.subscribe_session_event_head(session_id).await
    }

    pub async fn publish_event(self: &Arc<Self>, event: SessionEvent) {
        self.sessions.publish_event(self, event).await;
    }

    pub async fn remember_session_meta(&self, session: &Session) {
        self.sessions.remember_session_meta(session).await;
    }

    pub async fn refresh_session_head_cache(&self, session_id: SessionId) {
        self.sessions
            .refresh_session_head_cache(self, session_id)
            .await;
    }

    pub async fn ensure_scheduler(
        self: &Arc<Self>,
        session: Session,
    ) -> mpsc::Sender<crate::scheduler::SchedulerCommand> {
        self.sessions.ensure_scheduler(self, session).await
    }

    pub async fn scheduler_sender(
        &self,
        session_id: SessionId,
    ) -> Option<mpsc::Sender<crate::scheduler::SchedulerCommand>> {
        self.sessions.scheduler_sender(session_id).await
    }

    pub async fn is_running(&self, session_id: SessionId) -> bool {
        self.sessions.is_running(session_id).await
    }

    pub async fn list_running_sessions(&self) -> Vec<SessionId> {
        self.sessions.list_running_sessions().await
    }

    pub async fn set_running(&self, session_id: SessionId, running: bool) {
        self.sessions.set_running(session_id, running).await;
    }

    pub async fn cleanup_session(&self, session_id: SessionId) {
        self.sessions.cleanup_session(self, session_id).await;
    }
}

#[cfg(test)]
mod cache_sweep_tests {
    use super::{active_head_projection_should_flush, active_head_projection_wait_duration};
    use std::time::{Duration, Instant};

    #[test]
    fn active_head_projection_waits_for_debounce_or_max_flush() {
        let debounce = Duration::from_millis(200);
        let max_flush = Duration::from_millis(1500);
        let now = Instant::now();

        let wait = active_head_projection_wait_duration(
            now,
            now - Duration::from_millis(100),
            now - Duration::from_millis(100),
            debounce,
            max_flush,
        );
        assert_eq!(wait, Duration::from_millis(100));

        let wait = active_head_projection_wait_duration(
            now,
            now - Duration::from_millis(50),
            now - Duration::from_millis(1490),
            debounce,
            max_flush,
        );
        assert_eq!(wait, Duration::from_millis(10));
    }

    #[test]
    fn active_head_projection_flushes_on_idle_or_max() {
        let debounce = Duration::from_millis(200);
        let max_flush = Duration::from_millis(1500);
        let now = Instant::now();

        assert!(active_head_projection_should_flush(
            now,
            now - Duration::from_millis(250),
            now - Duration::from_millis(500),
            debounce,
            max_flush
        ));
        assert!(active_head_projection_should_flush(
            now,
            now - Duration::from_millis(50),
            now - Duration::from_millis(1600),
            debounce,
            max_flush
        ));
        assert!(!active_head_projection_should_flush(
            now,
            now - Duration::from_millis(50),
            now - Duration::from_millis(500),
            debounce,
            max_flush
        ));
    }
}
