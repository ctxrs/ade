use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, watch, Mutex};

use ctx_core::ids::{SessionId, TaskId};
use ctx_core::models::{
    Session, SessionEvent, SessionEventType, SessionHeadDelta, SessionHeadSnapshot,
    SessionTurnToolSummary, TaskDeltaKind,
};
use ctx_store::Store;
use ctx_workspace_active_snapshot::session_metadata_from_session;

use crate::daemon::state::{
    ActiveTaskRefreshEntry, AppState, SessionHeadCacheKey, SessionRuntime, TimedEntry,
};
use crate::order_seq::OrderSeqState;
use crate::scheduler::session_worker;

use super::head_projection::{
    activity_from_turn, build_session_summary_delta, derive_message_preview,
    derive_summary_activity, event_context_window, is_session_gap_notice, message_from_event,
    patch_turn_from_event, recompute_turn_tool_counts, resolve_projection_rev_for_stream_delta,
    should_include_session_metadata_in_head_delta, should_refresh_turn_from_store,
    turn_from_cached_head_for_read, turn_from_event,
};

const ACTIVE_TASK_REFRESH_DEBOUNCE_MS: u64 = 250;

impl SessionRuntime {
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

    pub async fn publish_event(&self, state: &Arc<AppState>, event: SessionEvent) {
        let tx = self.get_broadcaster(event.session_id).await;
        let _ = tx.send(event.clone());
        self.publish_session_event_head(event.session_id, event.seq)
            .await;
        self.update_workspace_active_snapshot_for_event(state, &event)
            .await;
    }

    async fn update_workspace_active_snapshot_for_event(
        &self,
        state: &Arc<AppState>,
        event: &SessionEvent,
    ) {
        if is_session_gap_notice(event) {
            return;
        }
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
            SessionEventType::AssistantChunk
                | SessionEventType::ThoughtChunk
                | SessionEventType::ContextWindowUpdate
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
            self.queue_workspace_task_delta_refresh(state, session.task_id)
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
        let mut tool_summaries: Vec<SessionTurnToolSummary> = Vec::new();
        let tool_event = matches!(
            event.event_type,
            SessionEventType::ToolCall
                | SessionEventType::ToolCallUpdate
                | SessionEventType::ToolResult
        );
        if tool_event {
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
        let mut turn = turn_from_event(event, message.as_ref());
        let prefers_cached_turn = tool_event
            || event_context_window(event).is_some()
            || should_refresh_turn_from_store(&event.event_type);
        if turn.is_none() && prefers_cached_turn {
            if let Some(turn_id) = event.turn_id {
                turn = turn_from_cached_head_for_read(state, event.session_id, turn_id).await;
            }
        }
        if turn.is_none() && (should_refresh_turn_from_store(&event.event_type) || tool_event) {
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
        if let Some(turn) = turn.as_mut() {
            patch_turn_from_event(turn, event);
            if !tool_summaries.is_empty() {
                recompute_turn_tool_counts(turn, &tool_summaries);
            }
        }

        let cached_replay_cursor = if stream_only {
            state
                .workspaces
                .workspace_active_snapshot
                .session_replay_cursor(session.workspace_id, event.session_id)
                .await
        } else {
            Default::default()
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
            || async {
                match state.store_for_session(event.session_id).await {
                    Ok(store) => store
                        .get_session_projection_rev(event.session_id)
                        .await
                        .ok(),
                    Err(_) => None,
                }
            },
        )
        .await;
        let state_rev = last_event_seq;

        let activity =
            derive_summary_activity(event).or_else(|| turn.as_ref().map(activity_from_turn));

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

        if let Some(summary_delta) = summary_delta {
            state
                .workspaces
                .workspace_active_snapshot
                .publish_session_summary_delta(session.workspace_id, summary_delta)
                .await;
        }
    }

    async fn queue_workspace_task_delta_refresh(&self, state: &Arc<AppState>, task_id: TaskId) {
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
                    .run_workspace_task_delta_refresh(state_clone, task_id)
                    .await;
            });
        }
    }

    async fn run_workspace_task_delta_refresh(&self, state: Arc<AppState>, task_id: TaskId) {
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

            match state.store_for_task(task_id).await {
                Ok(store) => match store.get_workspace_active_task_summary(task_id).await {
                    Ok(Some(summary)) => {
                        let _ = state
                            .emit_workspace_task_delta(summary.task, TaskDeltaKind::Updated)
                            .await;
                    }
                    Ok(None) => match store.get_task(task_id).await {
                        Ok(Some(task)) => {
                            let kind = if task.archived_at.is_some() {
                                TaskDeltaKind::Archived
                            } else {
                                TaskDeltaKind::Updated
                            };
                            let _ = state.emit_workspace_task_delta(task, kind).await;
                        }
                        Ok(None) => {}
                        Err(err) => {
                            tracing::warn!(
                                task_id = %task_id.0,
                                "workspace task delta refresh read failed: {err:?}"
                            );
                        }
                    },
                    Err(err) => {
                        tracing::warn!(
                            task_id = %task_id.0,
                            "workspace task delta refresh summary read failed: {err:?}"
                        );
                    }
                },
                Err(err) => {
                    tracing::warn!(
                        task_id = %task_id.0,
                        "workspace task delta refresh store lookup failed: {err:?}"
                    );
                }
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
        let head = match store.get_active_snapshot_head(session_id).await {
            Ok(Some(head)) => head,
            Ok(None) => {
                state
                    .workspaces
                    .workspace_active_snapshot
                    .remove_session(session_id)
                    .await;
                return;
            }
            Err(err) => {
                tracing::warn!(
                    session_id = %session_id.0,
                    "active session head cache refresh failed: {err:#}"
                );
                return;
            }
        };
        state
            .workspaces
            .workspace_active_snapshot
            .update_compact_session_head(head)
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
        tokio::spawn(session_worker(Arc::downgrade(state), session, rx));
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

    pub async fn cleanup_session(&self, state: &AppState, session_id: SessionId) {
        let workspace_id = state
            .global_store()
            .get_workspace_id_for_session(session_id)
            .await
            .ok()
            .flatten();
        if let Some(workspace_id) = workspace_id {
            state
                .workspaces
                .workspace_active_snapshot
                .remove_session_with_workspace_hint(workspace_id, session_id)
                .await;
        } else {
            state
                .workspaces
                .workspace_active_snapshot
                .remove_session(session_id)
                .await;
        }
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
            let mut set = self.running_sessions.lock().await;
            set.remove(&session_id);
        }
        {
            let mut pins = self.session_pins.lock().await;
            pins.remove(&session_id);
        }
        {
            let mut cache = self.session_meta_cache.lock().await;
            cache.remove(&session_id);
        }
    }
}
