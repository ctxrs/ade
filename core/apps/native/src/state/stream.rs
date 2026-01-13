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

use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::{
    SessionCatchupSummary, SessionEvent, SessionHeadDelta, WorkspaceCatchupClientMessage,
    WorkspaceCatchupEvent, WorkspaceCatchupSessionSubscription,
};

use super::ShellView;
use super::session::{clear_placeholder_messages_in, SessionThreadCache};
use super::super::models::{message_item_from_model, session_info_from_summary};

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
    Event(WorkspaceCatchupEvent),
}

const MAX_SESSION_EVENTS: usize = 200;

fn push_session_event_with_limit(events: &mut Vec<SessionEvent>, event: SessionEvent) {
    events.push(event);
    if events.len() > MAX_SESSION_EVENTS {
        let overflow = events.len() - MAX_SESSION_EVENTS;
        events.drain(0..overflow);
    }
}

fn apply_delta_to_cache(cache: &mut SessionThreadCache, delta: SessionHeadDelta) {
    if let Some(turn) = delta.turn {
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
        push_session_event_with_limit(&mut cache.session_events, event);
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

    fn build_stream_subscribe_message(&self) -> WorkspaceCatchupClientMessage {
        let mut session_ids = self
            .session_summary_map
            .keys()
            .copied()
            .collect::<Vec<_>>();
        session_ids.sort_by(|a, b| a.0.cmp(&b.0));
        let sessions = session_ids
            .into_iter()
            .map(|session_id| WorkspaceCatchupSessionSubscription {
                session_id,
                after_seq: self.session_last_event_seq.get(&session_id).copied(),
            })
            .collect();
        WorkspaceCatchupClientMessage::Subscribe {
            session_ids: Vec::new(),
            sessions,
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
        }
    }

    fn apply_workspace_event(&mut self, event: WorkspaceCatchupEvent, cx: &mut Context<Self>) {
        match event {
            WorkspaceCatchupEvent::TaskUpsert { task, .. } => {
                self.upsert_task_summary(task);
                self.send_stream_subscribe();
                self.maybe_mark_selected_task_read(cx);
                cx.notify();
            }
            WorkspaceCatchupEvent::TaskDelete { task_id, .. } => {
                self.remove_task(task_id);
                self.send_stream_subscribe();
                cx.notify();
            }
            WorkspaceCatchupEvent::TrackUpsert { track, .. } => {
                self.apply_track_summary(track);
                self.send_stream_subscribe();
                cx.notify();
            }
            WorkspaceCatchupEvent::SessionSummary { summary, .. } => {
                self.handle_session_summary(summary, cx);
            }
            WorkspaceCatchupEvent::SessionHeadDelta { delta, .. } => {
                self.apply_session_head_delta(*delta, cx);
            }
            WorkspaceCatchupEvent::SessionGap {
                session_id,
                after_seq,
                ..
            } => {
                self.handle_session_gap(session_id, after_seq, cx);
            }
            _ => {}
        }
    }

    fn handle_session_summary(&mut self, summary: SessionCatchupSummary, cx: &mut Context<Self>) {
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
            if let Some(cache) = self.session_thread_cache.get_mut(&session_id) {
                apply_delta_to_cache(cache, delta);
            }
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
            self.push_session_event(event);
        }

        if let Some(message) = message {
            self.push_message(message_item_from_model(&message), cx);
        } else {
            self.rebuild_thread_items();
        }

        self.cache_session_thread_state(session_id);
        cx.notify();
    }

    fn handle_session_gap(&mut self, session_id: SessionId, after_seq: i64, cx: &mut Context<Self>) {
        self.update_session_last_event_seq(session_id, after_seq);
        if self.is_session_selected(session_id) {
            self.resyncing_session = Some(session_id);
            self.load_session_details(session_id, cx);
        }
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
    mut subscribe_rx: watch::Receiver<WorkspaceCatchupClientMessage>,
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
            if let Some(event) = parse_workspace_stream_event(msg) {
                if !matches!(event, WorkspaceCatchupEvent::Ready { .. }) {
                    let _ = update_tx.send(StreamUpdate::Event(event));
                }
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
                            if let Some(event) = parse_workspace_stream_event(frame) {
                                if !matches!(event, WorkspaceCatchupEvent::Ready { .. }) {
                                    if update_tx.send(StreamUpdate::Event(event)).is_err() {
                                        return Ok(());
                                    }
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

fn parse_workspace_stream_event(message: WsMessage) -> Option<WorkspaceCatchupEvent> {
    let text = match message {
        WsMessage::Text(text) => text.to_string(),
        WsMessage::Binary(bytes) => String::from_utf8(bytes.to_vec()).ok()?,
        _ => return None,
    };
    serde_json::from_str::<WorkspaceCatchupEvent>(&text).ok()
}

async fn send_workspace_subscribe<S>(
    sink: &mut S,
    message: &WorkspaceCatchupClientMessage,
) -> Result<()>
where
    S: SinkExt<WsMessage> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let payload = serde_json::to_string(message)?;
    sink.send(WsMessage::Text(Utf8Bytes::from(payload))).await?;
    Ok(())
}
