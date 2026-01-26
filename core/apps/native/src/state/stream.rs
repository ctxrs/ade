use std::time::Duration;

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use gpui::{AsyncApp, Context, WeakEntity};
use gpui_tokio::Tokio;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message as WsMessage, Utf8Bytes},
};

use ctx_core::ids::{MessageId, SessionId, WorkspaceId};
use ctx_core::models::{
    MessageDelivery, SessionEvent, SessionEventType, SessionHeadDelta, SessionSnapshotSummary,
    SessionTurn, SessionTurnStatus,
    WorkspaceActiveSnapshotEvent, WorkspaceActiveSnapshotClientMessage,
    WorkspaceActiveSnapshotSessionSubscription, WorkspaceActiveSnapshotStreamMessage,
};
use serde_json::Value;

use super::ShellView;
use super::session::{
    clear_placeholder_messages_in, is_partial_event, strip_partial_turn, SessionThreadCache,
};
use super::super::models::{message_item_from_model, session_info_from_summary, MessageItem};

#[derive(Clone)]
pub(crate) enum StreamStatus {
    Idle,
    Connecting,
    Connected,
    Reconnecting { reason: Option<String> },
}

impl StreamStatus {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            StreamStatus::Idle => "Idle",
            StreamStatus::Connecting => "Connecting",
            StreamStatus::Connected => "Connected",
            StreamStatus::Reconnecting { .. } => "Reconnecting",
        }
    }

    pub(crate) fn detail(&self) -> Option<&str> {
        match self {
            StreamStatus::Reconnecting { reason } => reason.as_deref(),
            _ => None,
        }
    }
}

enum StreamUpdate {
    Status(StreamStatus),
    Event(WorkspaceActiveSnapshotEvent),
    HeadsBatch { snapshot_rev: i64, deltas: Vec<SessionHeadDelta> },
    ResetRequired { latest_rev: i64 },
}

const MAX_SESSION_EVENTS: usize = 200;

fn push_session_event_with_limit(events: &mut Vec<SessionEvent>, event: SessionEvent) {
    events.push(event);
    if events.len() > MAX_SESSION_EVENTS {
        let overflow = events.len() - MAX_SESSION_EVENTS;
        events.drain(0..overflow);
    }
}

fn turn_status_from_payload(event: &SessionEvent) -> Option<SessionTurnStatus> {
    let raw = event.payload_json.get("status").and_then(Value::as_str)?;
    match raw {
        "queued" => Some(SessionTurnStatus::Queued),
        "running" => Some(SessionTurnStatus::Running),
        "completed" => Some(SessionTurnStatus::Completed),
        "interrupted" => Some(SessionTurnStatus::Interrupted),
        "failed" => Some(SessionTurnStatus::Failed),
        _ => None,
    }
}

fn turn_status_from_event(event: &SessionEvent) -> Option<SessionTurnStatus> {
    if let Some(status) = turn_status_from_payload(event) {
        return Some(status);
    }
    match event.event_type {
        SessionEventType::TurnQueued => Some(SessionTurnStatus::Queued),
        SessionEventType::TurnStarted => Some(SessionTurnStatus::Running),
        SessionEventType::TurnFinished => Some(SessionTurnStatus::Completed),
        SessionEventType::TurnInterrupted => Some(SessionTurnStatus::Interrupted),
        SessionEventType::Done | SessionEventType::AssistantComplete => {
            Some(SessionTurnStatus::Completed)
        }
        SessionEventType::Error => Some(SessionTurnStatus::Failed),
        _ => None,
    }
}

fn apply_turn_event_to_turns(turns: &mut Vec<SessionTurn>, event: &SessionEvent) -> bool {
    let Some(turn_id) = event.turn_id else {
        return false;
    };
    let Some(turn) = turns.iter_mut().find(|turn| turn.turn_id == turn_id) else {
        return false;
    };
    let mut changed = false;
    if let Some(status) = turn_status_from_event(event) {
        turn.status = status;
        changed = true;
    }
    if let Some(metrics) = event.payload_json.get("context_window") {
        turn.metrics_json = Some(metrics.clone());
        changed = true;
    }
    if changed {
        turn.updated_at = event.created_at;
    }
    changed
}

fn message_id_from_event(event: &SessionEvent) -> Option<MessageId> {
    let id = event.payload_json.get("message_id").and_then(Value::as_str)?;
    uuid::Uuid::parse_str(id).ok().map(MessageId)
}

fn apply_queue_event_to_messages(messages: &mut Vec<MessageItem>, event: &SessionEvent) -> bool {
    let Some(message_id) = message_id_from_event(event) else {
        return false;
    };
    match event.event_type {
        SessionEventType::MessageQueueAdded => {
            if let Some(msg) = messages.iter_mut().find(|msg| msg.id == Some(message_id)) {
                if !matches!(msg.delivery, MessageDelivery::Queued) {
                    msg.delivery = MessageDelivery::Queued;
                    return true;
                }
            }
            false
        }
        SessionEventType::MessageQueueUpdated => {
            if let Some(msg) = messages.iter_mut().find(|msg| msg.id == Some(message_id)) {
                if !matches!(msg.delivery, MessageDelivery::Queued) {
                    msg.delivery = MessageDelivery::Queued;
                    return true;
                }
                return true;
            }
            false
        }
        SessionEventType::MessageQueuePromoted => {
            if let Some(msg) = messages.iter_mut().find(|msg| msg.id == Some(message_id)) {
                if matches!(msg.delivery, MessageDelivery::Queued) {
                    msg.delivery = MessageDelivery::Immediate;
                    return true;
                }
            }
            false
        }
        SessionEventType::MessageQueueRemoved => {
            let before = messages.len();
            messages.retain(|msg| msg.id != Some(message_id));
            before != messages.len()
        }
        _ => false,
    }
}

fn apply_delta_to_cache(cache: &mut SessionThreadCache, delta: SessionHeadDelta) {
    if let Some(turn) = delta.turn {
        let turn = strip_partial_turn(&turn);
        let mut found = false;
        for existing in &mut cache.session_turns {
            if existing.turn_id == turn.turn_id {
                *existing = turn.clone();
                found = true;
                break;
            }
        }
        if !found {
            cache.session_turns.push(turn);
            cache
                .session_turns
                .sort_by(|a, b| a.started_at.cmp(&b.started_at));
        }
    }

    if let Some(event) = delta.event {
        if !is_partial_event(&event) {
            push_session_event_with_limit(&mut cache.session_events, event.clone());
        }
        apply_turn_event_to_turns(&mut cache.session_turns, &event);
        apply_queue_event_to_messages(&mut cache.messages, &event);
    }

    if let Some(message) = delta.message {
        clear_placeholder_messages_in(&mut cache.messages);
        cache.messages.push(message_item_from_model(&message));
    }
}

impl ShellView {
    pub(super) fn stop_workspace_stream(&mut self) {
        if let Some(stop_tx) = self.stream_stop_tx.take() {
            let _ = stop_tx.send(true);
        }
        self.stream_subscribe_tx = None;
        self.stream_status = StreamStatus::Idle;
    }

    pub(super) fn start_workspace_stream(&mut self, workspace_id: WorkspaceId, cx: &mut Context<Self>) {
        let config = match ctx_client::resolve_daemon_config() {
            Ok(config) => config,
            Err(err) => {
                self.stream_status = StreamStatus::Reconnecting {
                    reason: Some(err.to_string()),
                };
                cx.notify();
                return;
            }
        };
        let client = match ctx_client::Client::new(config) {
            Ok(client) => client,
            Err(err) => {
                self.stream_status = StreamStatus::Reconnecting {
                    reason: Some(err.to_string()),
                };
                cx.notify();
                return;
            }
        };
        let ws_url = match client.workspace_stream_url(workspace_id) {
            Ok(url) => url,
            Err(err) => {
                self.stream_status = StreamStatus::Reconnecting {
                    reason: Some(err.to_string()),
                };
                cx.notify();
                return;
            }
        };

        let subscribe_message = self.build_stream_subscribe_message();
        let (subscribe_tx, subscribe_rx) = watch::channel(subscribe_message);
        let (stop_tx, stop_rx) = watch::channel(false);
        let (update_tx, mut update_rx) = mpsc::unbounded_channel();

        self.stream_subscribe_tx = Some(subscribe_tx);
        self.stream_stop_tx = Some(stop_tx);
        self.stream_status = StreamStatus::Connecting;
        cx.notify();

        let stream_task = Tokio::spawn_result(cx, async move {
            run_workspace_stream(ws_url, stop_rx, subscribe_rx, update_tx).await
        });

        cx.spawn(move |_: WeakEntity<ShellView>, _: &mut AsyncApp| async move {
            let _ = stream_task.await;
        })
        .detach();

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                while let Some(update) = update_rx.recv().await {
                    let _ = this.update(&mut cx, |view, cx| {
                        view.handle_stream_update(update, cx);
                    });
                }
            }
        })
        .detach();
    }

    fn build_stream_subscribe_message(&self) -> WorkspaceActiveSnapshotClientMessage {
        let mut session_ids = self
            .session_summary_map
            .keys()
            .copied()
            .collect::<Vec<_>>();
        session_ids.sort_by(|a, b| a.0.cmp(&b.0));
        let sessions = session_ids
            .into_iter()
            .map(|session_id| WorkspaceActiveSnapshotSessionSubscription {
                session_id,
                after_seq: self.session_last_event_seq.get(&session_id).copied(),
            })
            .collect();
        WorkspaceActiveSnapshotClientMessage::Subscribe {
            session_ids: Vec::new(),
            sessions,
            task_ids: Vec::new(),
            foreground_task_id: None,
            scope: None,
            include_active_heads: false,
        }
    }

    pub(super) fn send_stream_subscribe(&self) {
        if let Some(tx) = &self.stream_subscribe_tx {
            let _ = tx.send(self.build_stream_subscribe_message());
        }
    }

    fn handle_stream_update(&mut self, update: StreamUpdate, cx: &mut Context<Self>) {
        match update {
            StreamUpdate::Status(status) => {
                self.stream_status = status;
                cx.notify();
            }
            StreamUpdate::Event(event) => self.apply_workspace_event(event, cx),
            StreamUpdate::HeadsBatch { snapshot_rev, deltas } => {
                if snapshot_rev > 0 {
                    self.handle_snapshot_rev(snapshot_rev, false, cx);
                }
                for delta in deltas {
                    self.apply_session_head_delta(delta, cx);
                }
            }
            StreamUpdate::ResetRequired { latest_rev } => {
                if latest_rev > 0 {
                    self.workspace_snapshot_rev =
                        self.workspace_snapshot_rev.max(latest_rev);
                }
                self.refresh_active_snapshot(cx);
                self.prefetch_archived_head_window(cx);
            }
        }
    }

    fn apply_workspace_event(&mut self, event: WorkspaceActiveSnapshotEvent, cx: &mut Context<Self>) {
        let mut snapshot_rev = None;
        let mut archived_rev = None;
        let mut is_ready = false;
        match &event {
            WorkspaceActiveSnapshotEvent::Ready {
                snapshot_rev: active,
                archived_rev: archived,
                ..
            } => {
                snapshot_rev = Some(*active);
                archived_rev = Some(*archived);
                is_ready = true;
            }
            WorkspaceActiveSnapshotEvent::ActiveTaskUpsert {
                snapshot_rev: next_rev,
                ..
            }
            | WorkspaceActiveSnapshotEvent::ActiveTaskDelete {
                snapshot_rev: next_rev,
                ..
            }
            | WorkspaceActiveSnapshotEvent::SessionSummary {
                snapshot_rev: next_rev,
                ..
            }
            | WorkspaceActiveSnapshotEvent::SessionHeadDelta {
                snapshot_rev: next_rev,
                ..
            }
            | WorkspaceActiveSnapshotEvent::SessionHeadReset {
                snapshot_rev: next_rev,
                ..
            }
            | WorkspaceActiveSnapshotEvent::SessionGap {
                snapshot_rev: next_rev,
                ..
            }
            | WorkspaceActiveSnapshotEvent::WorktreeBootstrap {
                snapshot_rev: next_rev,
                ..
            } => {
                snapshot_rev = Some(*next_rev);
            }
            WorkspaceActiveSnapshotEvent::ArchivedTaskUpsert {
                archived_rev: next_rev,
                ..
            }
            | WorkspaceActiveSnapshotEvent::ArchivedTaskDelete {
                archived_rev: next_rev,
                ..
            } => {
                archived_rev = Some(*next_rev);
            }
        }
        if let Some(snapshot_rev) = snapshot_rev {
            self.handle_snapshot_rev(snapshot_rev, is_ready, cx);
        }
        if let Some(archived_rev) = archived_rev {
            self.handle_archived_rev(archived_rev, is_ready, cx);
        }

        match event {
            WorkspaceActiveSnapshotEvent::ActiveTaskUpsert { task, .. } => {
                self.upsert_active_task_summary(*task, cx);
                self.send_stream_subscribe();
                self.maybe_mark_selected_task_read(cx);
                cx.notify();
            }
            WorkspaceActiveSnapshotEvent::ActiveTaskDelete { task_id, .. } => {
                self.remove_task(task_id);
                self.send_stream_subscribe();
                cx.notify();
            }
            WorkspaceActiveSnapshotEvent::SessionSummary { summary, .. } => {
                self.handle_session_summary(*summary, cx);
            }
            WorkspaceActiveSnapshotEvent::SessionHeadDelta { delta, .. } => {
                self.apply_session_head_delta(*delta, cx);
            }
            WorkspaceActiveSnapshotEvent::SessionHeadReset { head, .. } => {
                self.handle_session_gap(head.session.id, head.last_event_seq, cx);
            }
            WorkspaceActiveSnapshotEvent::SessionGap {
                session_id,
                after_seq,
                ..
            } => {
                self.handle_session_gap(session_id, after_seq, cx);
            }
            WorkspaceActiveSnapshotEvent::ArchivedTaskUpsert { task, snapshot, .. } => {
                self.upsert_archived_task_summary(*task, snapshot.map(|snapshot| *snapshot), cx);
                cx.notify();
            }
            WorkspaceActiveSnapshotEvent::ArchivedTaskDelete { task_id, .. } => {
                self.remove_task(task_id);
                cx.notify();
            }
            WorkspaceActiveSnapshotEvent::Ready { .. } => {}
            _ => {}
        }
    }

    fn handle_snapshot_rev(&mut self, snapshot_rev: i64, is_ready: bool, cx: &mut Context<Self>) {
        if self.workspace_snapshot_rev > 0 {
            if snapshot_rev < self.workspace_snapshot_rev {
                self.workspace_snapshot_rev = snapshot_rev;
                self.refresh_active_snapshot(cx);
                self.prefetch_archived_head_window(cx);
                return;
            }
            if snapshot_rev > self.workspace_snapshot_rev + 1
                || (is_ready && snapshot_rev != self.workspace_snapshot_rev)
            {
                self.refresh_active_snapshot(cx);
                self.prefetch_archived_head_window(cx);
            }
        }
        self.workspace_snapshot_rev = snapshot_rev;
    }

    fn handle_archived_rev(&mut self, archived_rev: i64, is_ready: bool, cx: &mut Context<Self>) {
        if self.archived_snapshot_rev > 0 {
            if archived_rev < self.archived_snapshot_rev {
                self.archived_snapshot_rev = archived_rev;
                self.prefetch_archived_head_window(cx);
                return;
            }
            if archived_rev > self.archived_snapshot_rev + 1
                || (is_ready && archived_rev != self.archived_snapshot_rev)
            {
                self.prefetch_archived_head_window(cx);
            }
        }
        self.archived_snapshot_rev = archived_rev;
    }

    fn handle_session_summary(&mut self, summary: SessionSnapshotSummary, cx: &mut Context<Self>) {
        let session_id = summary.session.id;
        let is_new = !self.session_summary_map.contains_key(&session_id);
        self.apply_session_summary(summary);
        if self.is_session_selected(session_id) {
            if let Some(summary) = self.session_summary_map.get(&session_id) {
                self.session = session_info_from_summary(summary);
            }
        }

        if is_new {
            self.send_stream_subscribe();
        }

        self.maybe_mark_selected_task_read(cx);
        cx.notify();
    }

    fn apply_session_head_delta(&mut self, delta: SessionHeadDelta, cx: &mut Context<Self>) {
        let session_id = delta.session_id;
        self.update_session_last_event_seq(session_id, delta.last_event_seq);
        if !self.is_session_selected(session_id) {
            let cache = self
                .session_thread_cache
                .entry(session_id)
                .or_insert_with(SessionThreadCache::default);
            apply_delta_to_cache(cache, delta);
            self.persist_cached_session_head(session_id, cx);
            return;
        }

        let SessionHeadDelta {
            event,
            turn,
            message,
            ..
        } = delta;

        if let Some(turn) = turn {
            let mut found = false;
            for existing in &mut self.session_turns {
                if existing.turn_id == turn.turn_id {
                    *existing = turn.clone();
                    found = true;
                    break;
                }
            }
            if !found {
                self.session_turns.push(turn);
                self.session_turns
                    .sort_by(|a, b| a.started_at.cmp(&b.started_at));
            }
        }

        if let Some(event) = event {
            if matches!(event.event_type, SessionEventType::ArtifactsSet) {
                self.apply_artifacts_set_event(session_id, &event, cx);
            }
            apply_turn_event_to_turns(&mut self.session_turns, &event);
            apply_queue_event_to_messages(&mut self.messages, &event);
            self.push_session_event(event);
        }

        if let Some(message) = message {
            self.push_message(message_item_from_model(&message), cx);
        } else {
            self.rebuild_thread_items();
        }

        self.cache_session_thread_state(session_id);
        self.persist_cached_session_head(session_id, cx);
        cx.notify();
    }

    fn handle_session_gap(&mut self, session_id: SessionId, after_seq: i64, cx: &mut Context<Self>) {
        self.update_session_last_event_seq(session_id, after_seq);
        if self.is_session_selected(session_id) {
            self.resyncing_session = Some(session_id);
            self.load_session_details(session_id, cx);
            return;
        }
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let head = client
                .get_session_head(session_id, Some(40), Some(false))
                .await
                .ok();
            Ok((session_id, head))
        });

        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let result = task.await;
                this.update(&mut cx, |view, cx| {
                    let Ok((session_id, Some(head))) = result else {
                        return;
                    };
                    let summary = view.session_summary_map.get(&session_id).cloned();
                    if let Some(summary) = summary {
                        view.cache_session_snapshot(&summary, &head, cx);
                    }
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn push_session_event(&mut self, event: SessionEvent) {
        self.session_events.push(event);
        if self.session_events.len() > MAX_SESSION_EVENTS {
            let overflow = self.session_events.len() - MAX_SESSION_EVENTS;
            self.session_events.drain(0..overflow);
        }
    }
}

async fn run_workspace_stream(
    ws_url: String,
    mut stop_rx: watch::Receiver<bool>,
    mut subscribe_rx: watch::Receiver<WorkspaceActiveSnapshotClientMessage>,
    update_tx: mpsc::UnboundedSender<StreamUpdate>,
) -> Result<()> {
    let mut backoff = Duration::from_secs(1);
    loop {
        if *stop_rx.borrow() {
            let _ = update_tx.send(StreamUpdate::Status(StreamStatus::Idle));
            return Ok(());
        }

        if update_tx
            .send(StreamUpdate::Status(StreamStatus::Connecting))
            .is_err()
        {
            return Ok(());
        }

        let (socket, _) = match connect_async(&ws_url).await {
            Ok(connection) => connection,
            Err(err) => {
                let _ = update_tx.send(StreamUpdate::Status(StreamStatus::Reconnecting {
                    reason: Some(err.to_string()),
                }));
                tokio::time::sleep(backoff).await;
                backoff = (backoff + backoff).min(Duration::from_secs(10));
                continue;
            }
        };

        backoff = Duration::from_secs(1);
        let (mut write, mut read) = socket.split();

        let ready = tokio::select! {
            _ = stop_rx.changed() => {
                let _ = update_tx.send(StreamUpdate::Status(StreamStatus::Idle));
                return Ok(());
            }
            msg = read.next() => msg,
        };
        if let Some(Ok(msg)) = ready {
            if let Some(update) = parse_workspace_stream_message(msg) {
                let _ = update_tx.send(update);
            }
        } else {
            let _ = update_tx.send(StreamUpdate::Status(StreamStatus::Reconnecting {
                reason: Some("stream closed".to_string()),
            }));
            tokio::time::sleep(backoff).await;
            backoff = (backoff + backoff).min(Duration::from_secs(10));
            continue;
        }

        let _ = update_tx.send(StreamUpdate::Status(StreamStatus::Connected));
        let initial_subscribe = subscribe_rx.borrow().clone();
        if send_workspace_subscribe(&mut write, &initial_subscribe)
            .await
            .is_err()
        {
            let _ = update_tx.send(StreamUpdate::Status(StreamStatus::Reconnecting {
                reason: Some("subscribe failed".to_string()),
            }));
            tokio::time::sleep(backoff).await;
            backoff = (backoff + backoff).min(Duration::from_secs(10));
            continue;
        }

        loop {
            tokio::select! {
                _ = stop_rx.changed() => {
                    let _ = update_tx.send(StreamUpdate::Status(StreamStatus::Idle));
                    return Ok(());
                }
                _ = subscribe_rx.changed() => {
                    let message = subscribe_rx.borrow().clone();
                    if send_workspace_subscribe(&mut write, &message)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(frame)) => {
                            if let Some(update) = parse_workspace_stream_message(frame) {
                                if update_tx.send(update).is_err() {
                                    return Ok(());
                                }
                            }
                        }
                        Some(Err(_)) | None => break,
                    }
                }
            }
        }

        let _ = update_tx.send(StreamUpdate::Status(StreamStatus::Reconnecting {
            reason: Some("stream disconnected".to_string()),
        }));
        tokio::time::sleep(backoff).await;
        backoff = (backoff + backoff).min(Duration::from_secs(10));
    }
}

fn parse_workspace_stream_message(message: WsMessage) -> Option<StreamUpdate> {
    let text = match message {
        WsMessage::Text(text) => text.to_string(),
        WsMessage::Binary(bytes) => String::from_utf8(bytes.to_vec()).ok()?,
        _ => return None,
    };
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    if let Ok(message) = serde_json::from_value::<WorkspaceActiveSnapshotStreamMessage>(value.clone())
    {
        match message {
            WorkspaceActiveSnapshotStreamMessage::ResetRequired { latest_rev } => {
                return Some(StreamUpdate::ResetRequired { latest_rev })
            }
            WorkspaceActiveSnapshotStreamMessage::Event { event, .. } => {
                return Some(StreamUpdate::Event(event))
            }
            WorkspaceActiveSnapshotStreamMessage::HeadsBatch {
                snapshot_rev,
                deltas,
                ..
            } => {
                return Some(StreamUpdate::HeadsBatch {
                    snapshot_rev,
                    deltas,
                })
            }
            WorkspaceActiveSnapshotStreamMessage::Snapshot { .. } => return None,
        }
    }
    if value
        .get("type")
        .and_then(|v| v.as_str())
        .is_some_and(|t| t == "reset_required")
    {
        let latest_rev = value
            .get("latest_rev")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        return Some(StreamUpdate::ResetRequired { latest_rev });
    }
    serde_json::from_value::<WorkspaceActiveSnapshotEvent>(value)
        .ok()
        .map(StreamUpdate::Event)
}

async fn send_workspace_subscribe<S>(
    sink: &mut S,
    message: &WorkspaceActiveSnapshotClientMessage,
) -> Result<()>
where
    S: SinkExt<WsMessage> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let payload = serde_json::to_string(message)?;
    sink.send(WsMessage::Text(Utf8Bytes::from(payload))).await?;
    Ok(())
}
