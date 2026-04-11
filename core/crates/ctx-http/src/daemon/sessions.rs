use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, watch, Mutex};

use ctx_core::ids::{SessionId, TaskId};
use ctx_core::models::{
    Message, MessageAttachment, MessageDelivery, MessageRole, Session, SessionActivityState,
    SessionEvent, SessionEventType, SessionHeadDelta, SessionHeadSnapshot, SessionSummaryDelta,
    SessionTurn, SessionTurnStatus, SessionTurnToolSummary,
};
use ctx_store::Store;

use crate::order_seq::OrderSeqState;
use crate::scheduler::session_worker;
use ctx_workspace_active_snapshot::session_metadata_from_session;

use super::state::{
    ActiveTaskRefreshEntry, AppState, SessionHeadCacheKey, SessionRuntime, TimedEntry,
};

const ACTIVE_TASK_REFRESH_DEBOUNCE_MS: u64 = 250;

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

fn derive_message_preview(content: &str) -> String {
    let trimmed = content.trim();
    let line = trimmed.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return String::new();
    }
    const MAX_CHARS: usize = 160;
    let mut out: String = line.chars().take(MAX_CHARS).collect();
    if line.chars().count() > MAX_CHARS {
        out.push_str("...");
    }
    out
}

fn event_context_window(event: &SessionEvent) -> Option<serde_json::Value> {
    event.payload_json.get("context_window").cloned()
}

fn is_session_gap_notice(event: &SessionEvent) -> bool {
    matches!(event.event_type, SessionEventType::Notice)
        && event
            .payload_json
            .get("kind")
            .and_then(|value| value.as_str())
            .is_some_and(|kind| kind == "session_gap")
}

fn turn_status_from_finished_event(event: &SessionEvent) -> SessionTurnStatus {
    event
        .payload_json
        .get("status")
        .and_then(|value| serde_json::from_value::<SessionTurnStatus>(value.clone()).ok())
        .unwrap_or(SessionTurnStatus::Completed)
}

fn patch_turn_from_event(turn: &mut SessionTurn, event: &SessionEvent) {
    match event.event_type {
        SessionEventType::TurnQueued => {
            turn.status = SessionTurnStatus::Queued;
        }
        SessionEventType::TurnStarted => {
            turn.status = SessionTurnStatus::Running;
        }
        SessionEventType::Done => {
            turn.status = SessionTurnStatus::Completed;
            turn.end_seq = Some(event.seq);
        }
        SessionEventType::TurnFinished => {
            turn.status = turn_status_from_finished_event(event);
            turn.end_seq = Some(event.seq);
        }
        SessionEventType::TurnInterrupted => {
            turn.status = SessionTurnStatus::Interrupted;
            turn.end_seq = Some(event.seq);
        }
        SessionEventType::Error => {
            turn.status = SessionTurnStatus::Failed;
            turn.end_seq = Some(event.seq);
        }
        _ => {}
    }
    if let Some(metrics_json) = event_context_window(event) {
        turn.metrics_json = Some(metrics_json);
    }
    turn.updated_at = event.created_at;
}

fn derive_summary_activity(event_type: &SessionEventType) -> Option<SessionActivityState> {
    match event_type {
        SessionEventType::TurnQueued => Some(SessionActivityState {
            is_working: false,
            last_turn_status: Some(SessionTurnStatus::Queued),
        }),
        SessionEventType::TurnStarted => Some(SessionActivityState {
            is_working: true,
            last_turn_status: Some(SessionTurnStatus::Running),
        }),
        SessionEventType::TurnFinished | SessionEventType::Done => Some(SessionActivityState {
            is_working: false,
            last_turn_status: Some(SessionTurnStatus::Completed),
        }),
        SessionEventType::TurnInterrupted => Some(SessionActivityState {
            is_working: false,
            last_turn_status: Some(SessionTurnStatus::Interrupted),
        }),
        SessionEventType::Error => Some(SessionActivityState {
            is_working: false,
            last_turn_status: Some(SessionTurnStatus::Failed),
        }),
        _ => None,
    }
}

fn activity_from_turn(turn: &SessionTurn) -> SessionActivityState {
    match turn.status {
        SessionTurnStatus::Queued => SessionActivityState {
            is_working: false,
            last_turn_status: Some(SessionTurnStatus::Queued),
        },
        SessionTurnStatus::Running => SessionActivityState {
            is_working: true,
            last_turn_status: Some(SessionTurnStatus::Running),
        },
        SessionTurnStatus::Completed => SessionActivityState {
            is_working: false,
            last_turn_status: Some(SessionTurnStatus::Completed),
        },
        SessionTurnStatus::Interrupted => SessionActivityState {
            is_working: false,
            last_turn_status: Some(SessionTurnStatus::Interrupted),
        },
        SessionTurnStatus::Failed => SessionActivityState {
            is_working: false,
            last_turn_status: Some(SessionTurnStatus::Failed),
        },
    }
}

fn should_include_session_metadata_in_head_delta(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::Init | SessionEventType::Notice
    )
}

fn recompute_turn_tool_counts(turn: &mut SessionTurn, tool_summaries: &[SessionTurnToolSummary]) {
    let mut total = 0_i64;
    let mut pending = 0_i64;
    let mut running = 0_i64;
    let mut completed = 0_i64;
    let mut failed = 0_i64;
    for summary in tool_summaries
        .iter()
        .filter(|summary| summary.turn_id == turn.turn_id)
    {
        total += 1;
        match summary.status.as_deref() {
            Some("running") | Some("in_progress") => running += 1,
            Some("completed") | Some("complete") | Some("ok") | Some("succeeded") => completed += 1,
            Some("failed") | Some("error") => failed += 1,
            _ => pending += 1,
        }
    }
    turn.tool_total = total;
    turn.tool_pending = pending;
    turn.tool_running = running;
    turn.tool_completed = completed;
    turn.tool_failed = failed;
}

fn build_session_summary_delta(
    session: &Session,
    activity: Option<SessionActivityState>,
    last_message_at: Option<chrono::DateTime<chrono::Utc>>,
    last_message_preview: Option<String>,
    last_event_seq: i64,
    projection_rev: i64,
    state_rev: i64,
) -> Option<SessionSummaryDelta> {
    if activity.is_none() && last_message_at.is_none() && last_message_preview.is_none() {
        return None;
    }

    Some(SessionSummaryDelta {
        session_id: session.id,
        task_id: session.task_id,
        activity,
        last_message_at,
        last_message_preview,
        last_event_seq: Some(last_event_seq),
        projection_rev: Some(projection_rev),
        state_rev: Some(state_rev),
    })
}

async fn resolve_projection_rev_for_stream_delta<F, Fut>(
    stream_only: bool,
    last_event_seq: i64,
    cached_projection_rev: i64,
    load_projection_rev: F,
) -> i64
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Option<i64>>,
{
    if stream_only {
        return cached_projection_rev.max(0);
    }
    load_projection_rev().await.unwrap_or(last_event_seq.max(0))
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

fn should_refresh_turn_from_store(event_type: &SessionEventType) -> bool {
    matches!(
        event_type,
        SessionEventType::TurnQueued
            | SessionEventType::TurnStarted
            | SessionEventType::Done
            | SessionEventType::TurnFinished
            | SessionEventType::TurnInterrupted
            | SessionEventType::Error
    )
}

async fn turn_from_cached_head_for_read(
    state: &Arc<AppState>,
    session_id: SessionId,
    turn_id: ctx_core::ids::TurnId,
) -> Option<SessionTurn> {
    state
        .workspaces
        .workspace_active_snapshot
        .get_cached_session_head_for_read(session_id)
        .await
        .and_then(|head| head.turns.into_iter().find(|turn| turn.turn_id == turn_id))
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
                // Prefer the cached in-memory head before the store for terminal deltas so
                // stream-time metrics_json survives store/projection lag.
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

        let activity = derive_summary_activity(&event.event_type)
            .or_else(|| turn.as_ref().map(activity_from_turn));

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
mod cache_sweep_tests;
#[cfg(test)]
mod event_filter_tests;
