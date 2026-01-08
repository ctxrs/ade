use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;
use gpui::{
    ClickEvent, Context, FocusHandle, KeyDownEvent, ListOffset, ListState, Window, px,
};
use gpui_tokio::Tokio;

use ctx_core::ids::{SessionId, WorkspaceId};
use ctx_core::models::{
    Artifact, SessionCatchupSummary, SessionEvent, SessionHead, SessionHeadDelta,
    SessionHistoryPage, WorkspaceCatchupClientMessage, WorkspaceCatchupEvent,
    WorkspaceCatchupSessionSubscription,
};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use crate::theme::ThemeColors;

use super::models::{
    build_message_items, message_item_from_model, session_info_from_head, session_info_from_summary,
    MessageItem, SessionInfo,
};
use super::workspace_summary::{
    catchup_counts,
    session_summaries as build_session_summaries,
    task_summaries as build_task_summaries,
    SessionSummaryItem,
    TaskSummaryItem,
};

#[derive(Clone)]
pub(crate) struct WorkspaceItem {
    pub(crate) id: WorkspaceId,
    pub(crate) name: String,
}

#[derive(Clone)]
pub(crate) struct ProviderItem {
    pub(crate) name: String,
    pub(crate) status: String,
}

struct InitialLoadResult {
    workspaces: Vec<WorkspaceItem>,
    providers: Vec<ProviderItem>,
}

struct WorkspaceLoadResult {
    catchup_active_total: Option<i64>,
    catchup_archived_total: Option<i64>,
    catchup_tasks: Vec<TaskSummaryItem>,
    catchup_sessions: Vec<SessionSummaryItem>,
    artifacts: Vec<Artifact>,
    session_summaries: Vec<SessionCatchupSummary>,
    session_summary_map: HashMap<SessionId, SessionCatchupSummary>,
    session_head: Option<SessionHead>,
    session_history: Option<SessionHistoryPage>,
    session_events: Vec<SessionEvent>,
}

struct SessionLoadResult {
    session_id: SessionId,
    session_head: Option<SessionHead>,
    session_history: Option<SessionHistoryPage>,
    session_events: Vec<SessionEvent>,
    artifacts: Vec<Artifact>,
}

#[derive(Clone, Copy)]
enum SessionControlAction {
    Interrupt,
    Cancel,
}

impl SessionControlAction {
    fn failure_message(self) -> &'static str {
        match self {
            SessionControlAction::Interrupt => "Unable to interrupt session.",
            SessionControlAction::Cancel => "Unable to cancel session.",
        }
    }
}

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
const MESSAGE_PLACEHOLDER_TEXTS: [&str; 3] = [
    "Loading session messages...",
    "No messages yet. Create one to begin.",
    "Unable to load session details.",
];

pub(crate) enum DataLoadState {
    Loading,
    Loaded,
    Error(String),
}

pub(crate) struct ShellView {
    pub(crate) colors: ThemeColors,
    pub(crate) base_url: String,
    pub(crate) is_dark: bool,
    pub(crate) workspaces: Vec<WorkspaceItem>,
    pub(crate) selected_workspace: Option<WorkspaceId>,
    pub(crate) providers: Vec<ProviderItem>,
    pub(crate) catchup_active_total: Option<i64>,
    pub(crate) catchup_archived_total: Option<i64>,
    pub(crate) tasks: Vec<TaskSummaryItem>,
    pub(crate) selected_task: Option<usize>,
    pub(crate) sessions: Vec<SessionSummaryItem>,
    pub(crate) selected_session: Option<usize>,
    pub(crate) messages: Vec<MessageItem>,
    pub(crate) artifacts: Vec<Artifact>,
    pub(crate) selected_artifact: Option<usize>,
    pub(crate) session_events: Vec<SessionEvent>,
    pub(crate) session_summary_map: HashMap<SessionId, SessionCatchupSummary>,
    pub(crate) session: SessionInfo,
    pub(crate) data_state: DataLoadState,
    pub(crate) composer_text: String,
    pub(crate) composer_focus: FocusHandle,
    pub(crate) message_list_state: ListState,
    pub(crate) message_list_len: usize,
    pub(crate) message_auto_follow: bool,
    pub(crate) new_message_count: usize,
    pub(crate) stream_status: StreamStatus,
    pub(crate) stream_subscribe_tx: Option<watch::Sender<WorkspaceCatchupClientMessage>>,
    pub(crate) stream_stop_tx: Option<watch::Sender<bool>>,
    pub(crate) session_last_event_seq: HashMap<SessionId, i64>,
    pub(crate) message_list_handler_set: bool,
}

impl ShellView {
    pub(crate) fn toggle_theme(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_dark = !self.is_dark;
        self.colors = super::load_theme_colors(self.is_dark);
        cx.notify();
    }

    pub(crate) fn focus_composer(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        self.composer_focus.focus(window);
    }

    pub(crate) fn on_composer_key_down(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "backspace" {
            self.composer_text.pop();
            cx.notify();
            return;
        }

        if event.keystroke.key == "enter" || event.keystroke.key == "tab" {
            return;
        }

        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }

        if let Some(text) = event.keystroke.key_char.as_ref() {
            if text != "\n" && text != "\r" && text != "\t" {
                self.composer_text.push_str(text);
                cx.notify();
            }
        }
    }

    pub(crate) fn on_send_click(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.send_composer_message(cx);
    }

    pub(crate) fn on_interrupt_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_session_control(SessionControlAction::Interrupt, cx);
    }

    pub(crate) fn on_cancel_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_session_control(SessionControlAction::Cancel, cx);
    }

    pub(crate) fn on_new_messages_click(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.message_auto_follow = true;
        self.new_message_count = 0;
        self.scroll_messages_to_bottom();
        cx.notify();
    }

    fn send_composer_message(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.selected_session_id() else {
            return;
        };
        let content = self.composer_text.trim().to_string();
        if content.is_empty() {
            return;
        }

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let request = ctx_client::PostMessageRequest {
                content,
                delivery: None,
                attachments: Vec::new(),
            };
            client.post_message(session_id, &request).await?;
            Ok(session_id)
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                match result {
                    Ok(session_id) => {
                        view.composer_text.clear();
                        view.load_session_details(session_id, cx);
                    }
                    Err(_) => {
                        view.push_message(MessageItem::new(
                            "assistant",
                            "Unable to send message.",
                        ));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn request_session_control(
        &mut self,
        action: SessionControlAction,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.selected_session_id() else {
            return;
        };

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            match action {
                SessionControlAction::Interrupt => client.interrupt_session(session_id).await?,
                SessionControlAction::Cancel => client.cancel_session(session_id).await?,
            }
            Ok(session_id)
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                match result {
                    Ok(session_id) => {
                        view.load_session_details(session_id, cx);
                    }
                    Err(_) => {
                        view.push_message(MessageItem::new("assistant", action.failure_message()));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn ensure_message_list_handler(&mut self, cx: &mut Context<Self>) {
        if self.message_list_handler_set {
            return;
        }
        let view_handle = cx.entity();
        self.message_list_state
            .set_scroll_handler(move |event, _window, cx| {
                let _ = view_handle.update(cx, |view, cx| {
                    let auto_follow = !event.is_scrolled;
                    if view.message_auto_follow != auto_follow
                        || (auto_follow && view.new_message_count > 0)
                    {
                        view.message_auto_follow = auto_follow;
                        if auto_follow {
                            view.new_message_count = 0;
                        }
                        cx.notify();
                    }
                });
            });
        self.message_list_handler_set = true;
    }

    fn replace_messages(&mut self, messages: Vec<MessageItem>) {
        self.messages = messages;
        self.message_list_len = self.messages.len();
        self.message_list_state.reset(self.message_list_len);
        self.message_auto_follow = true;
        self.new_message_count = 0;
    }

    fn clear_placeholder_messages(&mut self) {
        if self.messages.len() != 1 {
            return;
        }
        let content = self.messages[0].content.as_str();
        if MESSAGE_PLACEHOLDER_TEXTS
            .iter()
            .any(|placeholder| placeholder == &content)
        {
            self.messages.clear();
            self.message_list_state.reset(0);
            self.message_list_len = 0;
        }
    }

    fn push_message(&mut self, message: MessageItem) {
        self.clear_placeholder_messages();
        let old_len = self.message_list_len;
        self.messages.push(message);
        let new_len = self.messages.len();
        if new_len != old_len {
            self.message_list_state.splice(old_len..old_len, new_len - old_len);
            self.message_list_len = new_len;
        }
        if self.message_auto_follow {
            self.scroll_messages_to_bottom();
        } else {
            self.new_message_count = self.new_message_count.saturating_add(new_len - old_len);
        }
    }

    fn scroll_messages_to_bottom(&mut self) {
        let count = self.messages.len();
        self.message_list_state.scroll_to(ListOffset {
            item_ix: count,
            offset_in_item: px(0.0),
        });
    }

    fn update_session_last_event_seq(&mut self, session_id: SessionId, seq: i64) {
        self.session_last_event_seq.insert(session_id, seq);
    }

    fn selected_session_id(&self) -> Option<SessionId> {
        self.selected_session
            .and_then(|index| self.sessions.get(index))
            .map(|summary| summary.session_id)
    }

    pub(crate) fn select_session(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(summary) = self.sessions.get(index) else {
            return;
        };
        let session_id = summary.session_id;
        self.selected_session = Some(index);
        self.session = self
            .session_summary_map
            .get(&session_id)
            .map(session_info_from_summary)
            .unwrap_or_else(SessionInfo::placeholder);
        self.replace_messages(vec![MessageItem::new(
            "assistant",
            "Loading session messages...",
        )]);
        self.artifacts.clear();
        self.session_events.clear();
        self.selected_artifact = None;
        cx.notify();
        self.load_session_details(session_id, cx);
    }

    pub(crate) fn select_artifact(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.artifacts.len() {
            return;
        }
        self.selected_artifact = Some(index);
        cx.notify();
    }

    fn load_session_details(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let session_head = client
                .get_session_head(session_id, Some(40), Some(false))
                .await
                .ok();
            let session_history = client
                .get_session_history(session_id, None, Some(60))
                .await
                .ok();
            let session_events = client
                .get_session_events(session_id, None, None, Some(40))
                .await
                .ok()
                .map(|page| page.events)
                .unwrap_or_default();
            let artifacts = client
                .list_session_artifacts(session_id)
                .await
                .ok()
                .map(|items| items.into_iter().collect::<Vec<_>>())
                .unwrap_or_default();

            Ok(SessionLoadResult {
                session_id,
                session_head,
                session_history,
                session_events,
                artifacts,
            })
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                if !view.is_session_selected(session_id) {
                    return;
                }
                match result {
                    Ok(data) => {
                        view.session = data
                            .session_head
                            .as_ref()
                            .map(session_info_from_head)
                            .or_else(|| {
                                view.session_summary_map
                                    .get(&data.session_id)
                                    .map(session_info_from_summary)
                            })
                            .unwrap_or_else(SessionInfo::placeholder);
                        if let Some(index) = view.selected_session {
                            if let Some(summary) = view.sessions.get_mut(index) {
                                if summary.session_id == data.session_id {
                                    summary.status = view.session.status.clone();
                                    summary.title = view.session.title.clone();
                                }
                            }
                        }
                        let mut messages = build_message_items(
                            data.session_head.as_ref(),
                            data.session_history.as_ref(),
                        );
                        if messages.is_empty() {
                            messages.push(MessageItem::new(
                                "assistant",
                                "No messages yet. Create one to begin.",
                            ));
                        }
                        view.replace_messages(messages);
                        view.session_events = data.session_events;
                        if let Some(head) = data.session_head.as_ref() {
                            view.update_session_last_event_seq(head.session.id, head.last_event_seq);
                        } else if let Some(event) = view.session_events.last() {
                            view.update_session_last_event_seq(event.session_id, event.seq);
                        }
                        view.artifacts = data.artifacts;
                        view.selected_artifact = if view.artifacts.is_empty() {
                            None
                        } else {
                            Some(0)
                        };
                    }
                    Err(_) => {
                        view.replace_messages(vec![MessageItem::new(
                            "assistant",
                            "Unable to load session details.",
                        )]);
                        view.session_events.clear();
                        view.artifacts.clear();
                        view.selected_artifact = None;
                        if let Some(summary) = view.session_summary_map.get(&session_id) {
                            view.session = session_info_from_summary(summary);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn is_session_selected(&self, session_id: SessionId) -> bool {
        self.selected_session
            .and_then(|index| self.sessions.get(index))
            .map(|summary| summary.session_id)
            == Some(session_id)
    }

    fn reset_workspace_view(&mut self, message: &str) {
        self.tasks.clear();
        self.selected_task = None;
        self.sessions.clear();
        self.selected_session = None;
        self.replace_messages(vec![MessageItem::new("assistant", message)]);
        self.artifacts.clear();
        self.session_events.clear();
        self.selected_artifact = None;
        self.session_summary_map.clear();
        self.session_last_event_seq.clear();
        self.session = SessionInfo::placeholder();
        self.catchup_active_total = None;
        self.catchup_archived_total = None;
    }

    pub(crate) fn select_workspace(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspaces.get(index) else {
            return;
        };
        if self.selected_workspace == Some(workspace.id) {
            return;
        }
        self.load_workspace(workspace.id, cx);
    }

    pub(crate) fn start_data_load(&mut self, cx: &mut Context<Self>) {
        self.ensure_message_list_handler(cx);
        self.data_state = DataLoadState::Loading;
        self.reset_workspace_view("Loading workspace data...");
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let workspaces = client.list_workspaces().await?;
            let providers = client.list_providers().await.unwrap_or_default();
            let workspaces = workspaces
                .into_iter()
                .map(|workspace| WorkspaceItem {
                    id: workspace.id,
                    name: workspace.name,
                })
                .collect::<Vec<_>>();
            let providers = providers
                .into_iter()
                .map(|provider| ProviderItem {
                    name: provider.provider_id,
                    status: format!("{:?}", provider.health),
                })
                .collect::<Vec<_>>();
            Ok(InitialLoadResult {
                workspaces,
                providers,
            })
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                match result {
                    Ok(data) => {
                        view.workspaces = data.workspaces;
                        view.providers = data.providers;
                        if let Some(selected) = view.selected_workspace {
                            if !view.workspaces.iter().any(|ws| ws.id == selected) {
                                view.selected_workspace = None;
                            }
                        }
                        if view.selected_workspace.is_none() {
                            view.selected_workspace =
                                view.workspaces.first().map(|workspace| workspace.id);
                        }
                        if let Some(workspace_id) = view.selected_workspace {
                            view.load_workspace(workspace_id, cx);
                        } else {
                            view.reset_workspace_view("No workspaces yet.");
                            view.data_state = DataLoadState::Loaded;
                            cx.notify();
                        }
                    }
                    Err(err) => {
                        view.workspaces.clear();
                        view.providers.clear();
                        view.selected_workspace = None;
                        view.reset_workspace_view("Unable to load workspace list.");
                        view.data_state = DataLoadState::Error(err.to_string());
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn load_workspace(&mut self, workspace_id: WorkspaceId, cx: &mut Context<Self>) {
        self.selected_workspace = Some(workspace_id);
        self.stop_workspace_stream();
        self.data_state = DataLoadState::Loading;
        self.reset_workspace_view("Loading workspace data...");
        cx.notify();

        let task = Tokio::spawn_result(cx, async move {
            let config = ctx_client::resolve_daemon_config()?;
            let client = ctx_client::Client::new(config)?;
            let params = ctx_client::WorkspaceCatchupParams {
                limit: Some(20),
                ..Default::default()
            };
            let snapshot = client.get_workspace_catchup(workspace_id, &params).await?;
            let counts = catchup_counts(&snapshot);
            let catchup_active_total = Some(counts.active_total);
            let catchup_archived_total = counts.archived_total;
            let catchup_tasks = build_task_summaries(&snapshot);
            let catchup_sessions = build_session_summaries(&snapshot);
            let mut session_summaries = Vec::new();
            let mut session_summary_map = HashMap::new();
            let mut first_session_id = None;
            for task in &snapshot.active.tasks {
                for track in &task.tracks {
                    for session in &track.sessions {
                        if first_session_id.is_none() {
                            first_session_id = Some(session.session.id);
                        }
                        session_summaries.push(session.clone());
                        session_summary_map.insert(session.session.id, session.clone());
                    }
                }
            }
            let mut session_head = None;
            let mut session_history = None;
            let mut session_events = Vec::new();
            let mut artifacts = Vec::new();
            if let Some(session_id) = first_session_id {
                session_head = client
                    .get_session_head(session_id, Some(40), Some(false))
                    .await
                    .ok();
                session_history = client
                    .get_session_history(session_id, None, Some(60))
                    .await
                    .ok();
                session_events = client
                    .get_session_events(session_id, None, None, Some(40))
                    .await
                    .ok()
                    .map(|page| page.events)
                    .unwrap_or_default();
                artifacts = client
                    .list_session_artifacts(session_id)
                    .await
                    .ok()
                    .map(|items| items.into_iter().collect::<Vec<_>>())
                    .unwrap_or_default();
            }

            Ok(WorkspaceLoadResult {
                catchup_active_total,
                catchup_archived_total,
                catchup_tasks,
                catchup_sessions,
                artifacts,
                session_summaries,
                session_summary_map,
                session_head,
                session_history,
                session_events,
            })
        });

        cx.spawn(|this, cx| async move {
            let result = task.await;
            this.update(cx, |view, cx| {
                if view.selected_workspace != Some(workspace_id) {
                    return;
                }
                match result {
                    Ok(data) => {
                        view.tasks = data.catchup_tasks;
                        view.selected_task = if view.tasks.is_empty() { None } else { Some(0) };
                        view.sessions = data.catchup_sessions;
                        view.selected_session =
                            if view.sessions.is_empty() { None } else { Some(0) };
                        view.session_summary_map = data.session_summary_map;
                        view.session_last_event_seq.clear();
                        for summary in &data.session_summaries {
                            if let Some(seq) = summary.last_event_seq {
                                view.update_session_last_event_seq(summary.session.id, seq);
                            }
                        }
                        view.catchup_active_total = data.catchup_active_total;
                        view.catchup_archived_total = data.catchup_archived_total;
                        view.session = data
                            .session_head
                            .as_ref()
                            .map(session_info_from_head)
                            .or_else(|| data.session_summaries.first().map(session_info_from_summary))
                            .unwrap_or_else(SessionInfo::placeholder);
                        let mut messages = build_message_items(
                            data.session_head.as_ref(),
                            data.session_history.as_ref(),
                        );
                        if messages.is_empty() {
                            messages.push(MessageItem::new(
                                "assistant",
                                "No messages yet. Create one to begin.",
                            ));
                        }
                        view.replace_messages(messages);
                        view.session_events = data.session_events;
                        if let Some(head) = data.session_head.as_ref() {
                            view.update_session_last_event_seq(head.session.id, head.last_event_seq);
                        } else if let Some(event) = view.session_events.last() {
                            view.update_session_last_event_seq(event.session_id, event.seq);
                        }
                        view.artifacts = data.artifacts;
                        view.selected_artifact = if view.artifacts.is_empty() {
                            None
                        } else {
                            Some(0)
                        };
                        view.data_state = DataLoadState::Loaded;
                        view.start_workspace_stream(workspace_id, cx);
                    }
                    Err(err) => {
                        view.reset_workspace_view("Unable to load workspace data.");
                        view.data_state = DataLoadState::Error(err.to_string());
                        view.stream_status = StreamStatus::Idle;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn stop_workspace_stream(&mut self) {
        if let Some(stop_tx) = self.stream_stop_tx.take() {
            let _ = stop_tx.send(true);
        }
        self.stream_subscribe_tx = None;
        self.stream_status = StreamStatus::Idle;
    }

    fn start_workspace_stream(&mut self, workspace_id: WorkspaceId, cx: &mut Context<Self>) {
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

        cx.spawn(|_, _| async move {
            let _ = stream_task.await;
        })
        .detach();

        cx.spawn(|this, cx| async move {
            while let Some(update) = update_rx.recv().await {
                let _ = this.update(cx, |view, cx| {
                    view.handle_stream_update(update, cx);
                });
            }
        })
        .detach();
    }

    fn build_stream_subscribe_message(&self) -> WorkspaceCatchupClientMessage {
        let sessions = self
            .sessions
            .iter()
            .map(|session| WorkspaceCatchupSessionSubscription {
                session_id: session.session_id,
                after_seq: self
                    .session_last_event_seq
                    .get(&session.session_id)
                    .copied(),
            })
            .collect();
        WorkspaceCatchupClientMessage::Subscribe {
            session_ids: Vec::new(),
            sessions,
        }
    }

    fn send_stream_subscribe(&self) {
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
            WorkspaceCatchupEvent::SessionSummary { summary, .. } => {
                self.apply_session_summary(summary, cx);
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

    fn apply_session_summary(&mut self, summary: SessionCatchupSummary, cx: &mut Context<Self>) {
        let session_id = summary.session.id;
        let info = session_info_from_summary(&summary);
        let is_new = !self.session_summary_map.contains_key(&session_id);

        self.session_summary_map.insert(session_id, summary.clone());
        if let Some(seq) = summary.last_event_seq {
            self.update_session_last_event_seq(session_id, seq);
        }

        if let Some(item) = self
            .sessions
            .iter_mut()
            .find(|item| item.session_id == session_id)
        {
            item.title = info.title.clone();
            item.status = info.status.clone();
        } else {
            self.sessions.push(SessionSummaryItem {
                session_id,
                title: info.title.clone(),
                status: info.status.clone(),
            });
        }

        if self.is_session_selected(session_id) {
            self.session = info;
        }

        if is_new {
            self.send_stream_subscribe();
        }

        cx.notify();
    }

    fn apply_session_head_delta(&mut self, delta: SessionHeadDelta, cx: &mut Context<Self>) {
        self.update_session_last_event_seq(delta.session_id, delta.last_event_seq);
        if !self.is_session_selected(delta.session_id) {
            return;
        }

        if let Some(event) = delta.event {
            self.push_session_event(event);
        }

        if let Some(message) = delta.message {
            self.push_message(message_item_from_model(&message));
        }

        cx.notify();
    }

    fn handle_session_gap(
        &mut self,
        session_id: SessionId,
        after_seq: i64,
        cx: &mut Context<Self>,
    ) {
        self.update_session_last_event_seq(session_id, after_seq);
        if self.is_session_selected(session_id) {
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
                        Some(Ok(_)) => {}
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
        WsMessage::Text(text) => text,
        WsMessage::Binary(bytes) => String::from_utf8(bytes).ok()?,
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
    sink.send(WsMessage::Text(payload)).await?;
    Ok(())
}
